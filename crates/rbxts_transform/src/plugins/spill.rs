//! Luau's limit of 200 local registers per function, checked on every file a plugin emitted anything into, once the
//! file is complete (DESIGN §9: the compiler owns the check for all code, its own output included).
//!
//! Luau gives a register to every live local: those of enclosing blocks count while a nested block runs, a numeric
//! `for` adds its loop variable and a generic `for` its names. Whether a `local` statement takes registers can
//! depend on how the script is compiled: Luau folds a statement whose values are all constants (and no more values
//! than names) into constants that take none, but only from optimisation level 1 (Roblox's default for testing in
//! Studio; live games run at 2) and only at debug level 1 (at debug level 2, which Roblox does not document for
//! Studio or live games, nothing folds). So two counts are kept, per function and at the deepest point:
//! - **conservative**: every local name, folded or not;
//! - **certain**: only the locals that take a register at every optimisation and debug level: a statement with a
//!   value Luau cannot fold (a call that is not to a builtin, a table, a function, an index that is not a builtin
//!   constant's, an interpolated string, `...`, or one of these inside an operator) or more values than names, a
//!   `local function`, and loop variables (a numeric `for` with constant bounds may be unrolled).
//!
//! In the main chunk, a conservative count past the limit spills the locals plugins hoisted with `hoist_spillable`
//! into one table local (an unneeded spill is harmless). A certain count still past the limit is an error: the file
//! would build, then fail to load ("Out of local registers"). A conservative count past it is a warning: the file
//! loads only where Luau folds its constant locals. Every function of the file gets the same check (no spill).

use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_luau::{
    AnyIdentifier, BinaryOperator, ElseBody, Expr, IfStatement, InterpolatedStringPart, Stmt, TableField,
    VariableDeclaration, create,
};
use rbxts_plugin::PluginDiagnostic;
use tsrs_ast::Node;
use tsrs_core::P;

use super::backend::to_diagnostic;
use crate::classes::prereqs::temp_id;
use crate::classes::transform_state::TransformState;

/// The most local registers Luau allows in one function, the main chunk included.
pub const LUAU_MAX_LOCALS: u32 = 200;

/// Register counts: every local name (`conservative`), and the locals that take a register at every optimisation
/// and debug level (`certain`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Registers {
    conservative: u32,
    certain: u32,
}

impl Registers {
    fn add(self, other: Registers) -> Registers {
        Registers { conservative: self.conservative + other.conservative, certain: self.certain + other.certain }
    }

    fn max(self, other: Registers) -> Registers {
        Registers { conservative: self.conservative.max(other.conservative), certain: self.certain.max(other.certain) }
    }

    fn names(count: usize, certain: bool) -> Registers {
        let count = count as u32;
        Registers { conservative: count, certain: if certain { count } else { 0 } }
    }
}

/// Globals whose members or calls Luau may fold into constants (builtin libraries and functions).
fn is_builtin_global(name: &str) -> bool {
    matches!(
        name,
        "math"
            | "bit32"
            | "string"
            | "table"
            | "vector"
            | "buffer"
            | "utf8"
            | "os"
            | "coroutine"
            | "debug"
            | "type"
            | "typeof"
            | "select"
            | "rawget"
            | "rawequal"
            | "rawlen"
            | "tonumber"
            | "tostring"
            | "assert"
            | "unpack"
    )
}

/// A local's name in a scope: an identifier's text, or a temporary's id (temporaries never shadow one another).
#[derive(Clone, PartialEq, Eq, Hash)]
enum Name {
    Text(String),
    Temporary(u32),
}

impl Name {
    fn of(id: &AnyIdentifier) -> Name {
        match id {
            AnyIdentifier::Identifier(id) => Name::Text(id.name.clone()),
            AnyIdentifier::Temporary(id) => Name::Temporary(id.id),
        }
    }
}

/// Counts the registers of one function (or the main chunk), knowing which locals are in scope and whether each may
/// be a constant Luau folds.
struct Counter<'a> {
    /// Luau may fold constants (not at `--!optimize 0`).
    folds: bool,
    /// An identifier no enclosing scope declares is a global (the main chunk), not possibly an outer constant (a
    /// function, whose upvalues are not tracked).
    unknown_is_global: bool,
    /// Temporaries assigned anywhere in the file: never constants (their ids are unique, unlike names).
    assigned: &'a FxHashSet<u32>,
    /// The locals in scope, innermost last: whether each may be a folded constant.
    scopes: Vec<FxHashMap<Name, bool>>,
}

impl Counter<'_> {
    fn lookup(&self, name: &Name) -> Option<bool> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name).copied())
    }

    fn bind(&mut self, id: &AnyIdentifier, maybe_constant: bool) {
        self.scopes.last_mut().expect("a scope").insert(Name::of(id), maybe_constant);
    }

    fn is_builtin(&self, e: &Expr) -> bool {
        let global_builtin = |e: &Expr| match e {
            Expr::Identifier(id) => is_builtin_global(&id.name) && self.lookup(&Name::Text(id.name.clone())).is_none(),
            _ => false,
        };
        match e {
            Expr::PropertyAccess(access) => global_builtin(&access.expression),
            Expr::ComputedIndex(access) => global_builtin(&access.expression),
            e => global_builtin(e),
        }
    }

    /// `e` is a value Luau never folds into a constant, at any optimisation or debug level.
    fn never_constant(&self, e: &Expr) -> bool {
        match e {
            Expr::Call(call) => !self.is_builtin(&call.expression) || call.args.iter().any(|a| self.never_constant(a)),
            Expr::MethodCall(_)
            | Expr::Function(_)
            | Expr::Array(_)
            | Expr::Map(_)
            | Expr::Set(_)
            | Expr::MixedTable(_)
            | Expr::InterpolatedString(_)
            | Expr::VarArgs => true,
            Expr::PropertyAccess(_) | Expr::ComputedIndex(_) => !self.is_builtin(e),
            Expr::Parenthesized(p) => self.never_constant(&p.expression),
            Expr::Unary(u) => self.never_constant(&u.expression),
            // `false and f()` folds to `false`: only the left operand decides
            Expr::Binary(b) if matches!(b.operator, BinaryOperator::And | BinaryOperator::Or) => {
                self.never_constant(&b.left)
            }
            Expr::Binary(b) => self.never_constant(&b.left) || self.never_constant(&b.right),
            Expr::If(i) => {
                self.never_constant(&i.condition)
                    || (self.never_constant(&i.expression) && self.never_constant(&i.alternative))
            }
            // a local that may be a constant may be folded; a global never is (but for a builtin)
            Expr::Identifier(id) => match self.lookup(&Name::Text(id.name.clone())) {
                Some(maybe_constant) => !maybe_constant,
                None => self.unknown_is_global && !is_builtin_global(&id.name),
            },
            Expr::TemporaryIdentifier(id) => self.lookup(&Name::Temporary(id.id)).is_some_and(|maybe| !maybe),
            Expr::None
            | Expr::Nil
            | Expr::True
            | Expr::False
            | Expr::Number(_)
            | Expr::String(_)
            | Expr::LateString(_) => false,
        }
    }

    /// A `local` statement: all its names, certainly when Luau cannot fold the statement (it folds a whole statement
    /// or none of it). The names are bound after their values.
    fn declaration(&mut self, declaration: &VariableDeclaration) -> Registers {
        let certain = !self.folds
            || declaration.right.len() > declaration.left.len()
            || declaration.right.iter().any(|value| self.never_constant(value))
            || declaration
                .left
                .iter()
                .any(|id| matches!(id, AnyIdentifier::Temporary(t) if self.assigned.contains(&t.id)));
        for id in &declaration.left {
            self.bind(id, !certain);
        }
        Registers::names(declaration.left.len(), certain)
    }

    /// What a statement declares in its block (binding the names), and the most registers live inside its nested
    /// blocks, on top of `active`.
    fn statement(&mut self, statement: &Stmt, active: Registers) -> (Registers, Registers) {
        let inner = match statement {
            Stmt::Do(block) => self.block(&block.statements, active),
            Stmt::While(w) => self.block(&w.statements, active),
            Stmt::Repeat(r) => self.block(&r.statements, active),
            Stmt::If(i) => self.if_statement(i, active),
            Stmt::NumericFor(f) => {
                // constant bounds may be unrolled (-O2): the loop variable then takes no register
                let constant = |e: &Expr| matches!(e, Expr::Number(_));
                let unrollable =
                    self.folds && constant(&f.start) && constant(&f.end) && f.step.as_ref().is_none_or(constant);
                self.scopes.push(FxHashMap::default());
                self.bind(&f.id, false);
                let most = self.block(&f.statements, active.add(Registers::names(1, !unrollable)));
                self.scopes.pop();
                most
            }
            Stmt::For(f) => {
                self.scopes.push(FxHashMap::default());
                for id in &f.ids {
                    self.bind(id, false);
                }
                let most = self.block(&f.statements, active.add(Registers::names(f.ids.len(), true)));
                self.scopes.pop();
                most
            }
            _ => active,
        };
        let declared = match statement {
            Stmt::VariableDeclaration(declaration) => self.declaration(declaration),
            Stmt::FunctionDeclaration(declaration) if declaration.localize => {
                if let Ok(id) = AnyIdentifier::try_from(declaration.name.clone()) {
                    self.bind(&id, false);
                }
                Registers::names(1, true)
            }
            _ => Registers::default(),
        };
        (declared, inner)
    }

    fn if_statement(&mut self, statement: &IfStatement, active: Registers) -> Registers {
        let then = self.block(&statement.statements, active);
        match &statement.else_body {
            ElseBody::If(next) => then.max(self.if_statement(next, active)),
            ElseBody::Block(statements) => then.max(self.block(statements, active)),
        }
    }

    /// A nested block's pressure, on top of `active`.
    fn block(&mut self, statements: &[Stmt], active: Registers) -> Registers {
        self.scopes.push(FxHashMap::default());
        let (most, _) = self.top(statements, active);
        self.scopes.pop();
        most
    }

    /// A block's pressure in the current scope, and for each count (conservative, certain) the index of the
    /// statement where it first passes the limit.
    fn top(&mut self, statements: &[Stmt], start: Registers) -> (Registers, [Option<usize>; 2]) {
        let mut active = start;
        let mut most = active;
        let mut passed = [None; 2];
        for (i, statement) in statements.iter().enumerate() {
            let (declared, inner) = self.statement(statement, active);
            active = active.add(declared);
            let here = inner.max(active);
            for (slot, count) in passed.iter_mut().zip([here.conservative, here.certain]) {
                if slot.is_none() && count > LUAU_MAX_LOCALS {
                    *slot = Some(i);
                }
            }
            most = most.max(here);
        }
        (most, passed)
    }
}

/// The pressure of a function body (or the main chunk: no parameters, unknown names are globals).
fn pressure(
    statements: &[Stmt],
    parameters: &[AnyIdentifier],
    main_chunk: bool,
    folds: bool,
    assigned: &FxHashSet<u32>,
) -> (Registers, [Option<usize>; 2]) {
    let mut counter = Counter { folds, unknown_is_global: main_chunk, assigned, scopes: vec![FxHashMap::default()] };
    for id in parameters {
        counter.bind(id, false);
    }
    counter.top(statements, Registers::names(parameters.len(), true))
}

/// The temporaries assigned anywhere (an assignment's target, `function _t()`).
fn assigned_temporaries(statements: &[Stmt]) -> FxHashSet<u32> {
    fn in_statements(statements: &[Stmt], out: &mut FxHashSet<u32>) {
        for statement in statements {
            match statement {
                Stmt::Assignment(a) => {
                    for target in &a.left {
                        if let Expr::TemporaryIdentifier(t) = target {
                            out.insert(t.id);
                        }
                    }
                }
                Stmt::FunctionDeclaration(f) => {
                    if let (false, Expr::TemporaryIdentifier(t)) = (f.localize, &f.name) {
                        out.insert(t.id);
                    }
                }
                _ => {}
            }
            for_each_nested_block(statement, &mut |block| in_statements(block, out));
        }
    }
    let mut out = FxHashSet::default();
    in_statements(statements, &mut out);
    for function in functions(statements) {
        in_statements(function.body, &mut out);
    }
    out
}

/// Calls `f` with each block nested directly in a statement (not function bodies).
fn for_each_nested_block(statement: &Stmt, f: &mut impl FnMut(&[Stmt])) {
    match statement {
        Stmt::Do(block) => f(&block.statements),
        Stmt::While(w) => f(&w.statements),
        Stmt::Repeat(r) => f(&r.statements),
        Stmt::NumericFor(n) => f(&n.statements),
        Stmt::For(n) => f(&n.statements),
        Stmt::If(i) => {
            let mut current = Some(i.as_ref());
            while let Some(i) = current {
                f(&i.statements);
                current = match &i.else_body {
                    ElseBody::If(next) => Some(next.as_ref()),
                    ElseBody::Block(statements) => {
                        f(statements);
                        None
                    }
                };
            }
        }
        _ => {}
    }
}

/// The optimisation level a `--!optimize N` directive among the file's leading comments sets (1 by default).
fn optimize_level(statements: &[Stmt]) -> u32 {
    statements
        .iter()
        .map_while(|s| match s {
            Stmt::Comment(text) => Some(text),
            _ => None,
        })
        .find_map(|text| text.strip_prefix("!optimize").map(|level| level.trim().parse().unwrap_or(1)))
        .unwrap_or(1)
}

/// One function of the file: its registers on entry (parameters, `self`) and its body, with the index of the
/// main chunk's statement it is in.
struct Function<'a> {
    parameters: &'a [AnyIdentifier],
    /// `function a:m()` has `self` too.
    implicit_self: bool,
    body: &'a [Stmt],
    top: usize,
}

/// Every function of the file, nested ones included.
fn functions(statements: &[Stmt]) -> Vec<Function<'_>> {
    fn in_statements<'a>(statements: &'a [Stmt], top: Option<usize>, out: &mut Vec<Function<'a>>) {
        for (i, statement) in statements.iter().enumerate() {
            in_statement(statement, top.unwrap_or(i), out);
        }
    }
    fn function<'a>(
        parameters: &'a [AnyIdentifier],
        implicit_self: bool,
        body: &'a [Stmt],
        top: usize,
        out: &mut Vec<Function<'a>>,
    ) {
        out.push(Function { parameters, implicit_self, body, top });
        in_statements(body, Some(top), out);
    }
    fn in_statement<'a>(statement: &'a Stmt, top: usize, out: &mut Vec<Function<'a>>) {
        match statement {
            Stmt::Assignment(a) => a.left.iter().chain(&a.right).for_each(|e| in_expr(e, top, out)),
            Stmt::Break | Stmt::Continue | Stmt::Comment(_) => {}
            Stmt::Call(e) => in_expr(e, top, out),
            Stmt::Do(block) => in_statements(&block.statements, Some(top), out),
            Stmt::While(w) => {
                in_expr(&w.condition, top, out);
                in_statements(&w.statements, Some(top), out);
            }
            Stmt::Repeat(r) => {
                in_statements(&r.statements, Some(top), out);
                in_expr(&r.condition, top, out);
            }
            Stmt::If(i) => in_if(i, top, out),
            Stmt::NumericFor(f) => {
                in_expr(&f.start, top, out);
                in_expr(&f.end, top, out);
                if let Some(step) = &f.step {
                    in_expr(step, top, out);
                }
                in_statements(&f.statements, Some(top), out);
            }
            Stmt::For(f) => {
                in_expr(&f.expression, top, out);
                in_statements(&f.statements, Some(top), out);
            }
            Stmt::FunctionDeclaration(f) => function(&f.parameters, false, &f.statements, top, out),
            Stmt::MethodDeclaration(m) => function(&m.parameters, true, &m.statements, top, out),
            Stmt::VariableDeclaration(v) => v.right.iter().for_each(|e| in_expr(e, top, out)),
            Stmt::Return(r) => r.expressions.iter().for_each(|e| in_expr(e, top, out)),
        }
    }
    fn in_if<'a>(statement: &'a IfStatement, top: usize, out: &mut Vec<Function<'a>>) {
        in_expr(&statement.condition, top, out);
        in_statements(&statement.statements, Some(top), out);
        match &statement.else_body {
            ElseBody::If(next) => in_if(next, top, out),
            ElseBody::Block(statements) => in_statements(statements, Some(top), out),
        }
    }
    fn in_expr<'a>(e: &'a Expr, top: usize, out: &mut Vec<Function<'a>>) {
        match e {
            Expr::Function(f) => function(&f.parameters, false, &f.statements, top, out),
            Expr::ComputedIndex(c) => {
                in_expr(&c.expression, top, out);
                in_expr(&c.index, top, out);
            }
            Expr::PropertyAccess(p) => in_expr(&p.expression, top, out),
            Expr::Call(c) => {
                in_expr(&c.expression, top, out);
                c.args.iter().for_each(|e| in_expr(e, top, out));
            }
            Expr::MethodCall(m) => {
                in_expr(&m.expression, top, out);
                m.args.iter().for_each(|e| in_expr(e, top, out));
            }
            Expr::Parenthesized(p) => in_expr(&p.expression, top, out),
            Expr::Binary(b) => {
                in_expr(&b.left, top, out);
                in_expr(&b.right, top, out);
            }
            Expr::Unary(u) => in_expr(&u.expression, top, out),
            Expr::If(i) => {
                in_expr(&i.condition, top, out);
                in_expr(&i.expression, top, out);
                in_expr(&i.alternative, top, out);
            }
            Expr::InterpolatedString(s) => {
                for part in &s.parts {
                    if let InterpolatedStringPart::Expression(e) = part {
                        in_expr(e, top, out);
                    }
                }
            }
            Expr::Array(items) | Expr::Set(items) => items.iter().for_each(|e| in_expr(e, top, out)),
            Expr::Map(fields) => fields.iter().for_each(|f| {
                in_expr(&f.index, top, out);
                in_expr(&f.value, top, out);
            }),
            Expr::MixedTable(fields) => fields.iter().for_each(|f| match f {
                TableField::Value(e) => in_expr(e, top, out),
                TableField::Field(f) => {
                    in_expr(&f.index, top, out);
                    in_expr(&f.value, top, out);
                }
            }),
            Expr::Identifier(_)
            | Expr::TemporaryIdentifier(_)
            | Expr::None
            | Expr::Nil
            | Expr::False
            | Expr::True
            | Expr::Number(_)
            | Expr::String(_)
            | Expr::LateString(_)
            | Expr::VarArgs => {}
        }
    }
    let mut out = Vec::new();
    in_statements(statements, None, &mut out);
    out
}

/// Checks a complete file a plugin emitted anything into (`plugin_file.emitted`): its main chunk (spilling the
/// `hoist_spillable` locals when it may pass Luau's limit) and each of its functions. Files no plugin emitted into
/// are left as roblox-ts leaves them.
pub(crate) fn check_main_chunk(state: &mut TransformState<'_>, statements: &mut Vec<Stmt>) {
    if !state.plugin_file.emitted {
        return;
    }
    let folds = optimize_level(statements) != 0;
    let assigned = assigned_temporaries(statements);
    let (mut most, mut passed) = pressure(statements, &[], true, folds, &assigned);
    let mut spilled_at = None;
    if most.conservative > LUAU_MAX_LOCALS && !state.plugin_file.spillable.is_empty() {
        spilled_at = spill(state, statements);
        let assigned = assigned_temporaries(statements);
        (most, passed) = pressure(statements, &[], true, folds, &assigned);
    }
    let mut diagnostics = Vec::new();
    if let Some(diagnostic) = limit_diagnostic(most, passed, "its main chunk", |at| {
        at.and_then(|at| statement_node(state, at, spilled_at)).unwrap_or_else(|| state.source_file.as_node())
    }) {
        diagnostics.push(diagnostic);
    }
    for function in functions(statements) {
        let (mut most, _) = pressure(function.body, function.parameters, false, folds, &assigned);
        if function.implicit_self {
            most = most.add(Registers::names(1, true));
        }
        let anchor = || statement_node(state, function.top, spilled_at).unwrap_or_else(|| state.source_file.as_node());
        if let Some(diagnostic) = limit_diagnostic(most, [Some(0), Some(0)], "one of its functions", |_| anchor()) {
            diagnostics.push(diagnostic);
        }
    }
    for diagnostic in diagnostics {
        state.add_diagnostic(to_diagnostic(&diagnostic));
    }
}

/// The error (a certain count past the limit) or warning (a conservative one) for a function's pressure.
fn limit_diagnostic(
    most: Registers,
    passed: [Option<usize>; 2],
    what: &str,
    anchor: impl Fn(Option<usize>) -> P<Node>,
) -> Option<PluginDiagnostic> {
    if most.certain > LUAU_MAX_LOCALS {
        let count = most.certain;
        return Some(PluginDiagnostic::error(
            anchor(passed[1]),
            format!(
                "This file needs {count} local registers in {what}, and Luau allows at most {LUAU_MAX_LOCALS}: it \
                 would build, then fail to load (\"Out of local registers\")."
            ),
        ));
    }
    if most.conservative > LUAU_MAX_LOCALS {
        let count = most.conservative;
        return Some(PluginDiagnostic::warning(
            anchor(passed[0]),
            format!(
                "This file declares {count} locals live at once in {what} ({} of them take a register however it \
                 is compiled), and Luau allows at most {LUAU_MAX_LOCALS} registers: it loads only where Luau folds \
                 its constant locals (optimisation level 1 or 2 at debug level 1), and fails to load at \
                 --!optimize 0 or debug level 2.",
                most.certain
            ),
        ));
    }
    None
}

/// The top-level TS statement whose Luau holds the final statement `index`: the root ranges, moved by the
/// statements inserted in front of them since (`plugin_file.front_inserts`) and by the spill table.
fn statement_node(state: &TransformState<'_>, index: usize, spilled_at: Option<usize>) -> Option<P<Node>> {
    let shift = |mut i: usize| {
        for &(at, count) in &state.plugin_file.front_inserts {
            if i >= at {
                i += count;
            }
        }
        if let Some(at) = spilled_at
            && i >= at
        {
            i += 1;
        }
        i
    };
    state.plugin_file.root_ranges.iter().find(|range| shift(range.end) > index).map(|range| range.node)
}

/// The spilled hoists' table and fields.
struct Spill {
    table: Expr,
    fields: FxHashMap<u32, String>,
}

/// Moves every `hoist_spillable` local whose declaration is a statement of the main chunk into one table local:
/// `local _spill = {}` where the first was declared, `_spill.<hint> = value` for each, and every use of one
/// becomes `_spill.<hint>`. Returns where the table's declaration went.
fn spill(state: &mut TransformState<'_>, statements: &mut Vec<Stmt>) -> Option<usize> {
    let spillable: FxHashMap<u32, String> =
        state.plugin_file.spillable.iter().map(|hoist| (hoist.id, hoist.hint.clone())).collect();
    // the declarations, in file order
    let declarations: Vec<(usize, u32)> = statements
        .iter()
        .enumerate()
        .filter_map(|(i, statement)| match statement {
            Stmt::VariableDeclaration(declaration) => match declaration.left.as_slice() {
                [AnyIdentifier::Temporary(id)] if spillable.contains_key(&id.id) && declaration.right.len() == 1 => {
                    Some((i, id.id))
                }
                _ => None,
            },
            _ => None,
        })
        .collect();
    let &(first, _) = declarations.first()?;
    let table_id = temp_id(state.plugin_file.seam_hint(state.source_file, "spill"));
    let table = Expr::TemporaryIdentifier(table_id.clone());
    let mut fields = FxHashMap::default();
    let mut taken = FxHashSet::default();
    for &(_, id) in &declarations {
        let hint = &spillable[&id];
        let base = if rbxts_luau::is_valid_identifier(hint) { hint.clone() } else { "field".to_owned() };
        let field = (1..)
            .map(|n| if n == 1 { base.clone() } else { format!("{base}_{n}") })
            .find(|name| !taken.contains(name))
            .expect("a free field name");
        taken.insert(field.clone());
        fields.insert(id, field);
    }
    let spill = Spill { table, fields };
    for &(i, id) in &declarations {
        let Stmt::VariableDeclaration(declaration) = &mut statements[i] else { unreachable!("found above") };
        let value = declaration.right.pop().expect("one value");
        statements[i] = create::assignment(
            create::property(spill.table.clone(), spill.fields[&id].clone()),
            rbxts_luau::AssignmentOperator::Assign,
            value,
        );
    }
    let mut misplaced = false;
    rewrite_statements(statements, &spill, &mut misplaced);
    // the table first: `local _spill = {}` before the first field assignment
    statements.insert(first, create::local(table_id, Some(create::map([]))));
    if misplaced {
        let node: P<Node> = state.source_file.as_node();
        state.internal_error(
            node,
            "a hoist_spillable local was declared again by plugin code (as a parameter, loop variable or local), so \
             it cannot be spilled into a table",
        );
    }
    Some(first)
}

fn rewrite_statements(statements: &mut [Stmt], spill: &Spill, misplaced: &mut bool) {
    for statement in statements {
        rewrite_statement(statement, spill, misplaced);
    }
}

fn check_declared(ids: &[AnyIdentifier], spill: &Spill, misplaced: &mut bool) {
    for id in ids {
        if let AnyIdentifier::Temporary(t) = id
            && spill.fields.contains_key(&t.id)
        {
            *misplaced = true;
        }
    }
}

fn rewrite_statement(statement: &mut Stmt, spill: &Spill, misplaced: &mut bool) {
    match statement {
        Stmt::Assignment(assignment) => {
            for e in assignment.left.iter_mut().chain(assignment.right.iter_mut()) {
                rewrite_expr(e, spill, misplaced);
            }
        }
        Stmt::Break | Stmt::Continue | Stmt::Comment(_) => {}
        Stmt::Call(e) => rewrite_expr(e, spill, misplaced),
        Stmt::Do(block) => rewrite_statements(&mut block.statements, spill, misplaced),
        Stmt::While(w) => {
            rewrite_expr(&mut w.condition, spill, misplaced);
            rewrite_statements(&mut w.statements, spill, misplaced);
        }
        Stmt::Repeat(r) => {
            rewrite_statements(&mut r.statements, spill, misplaced);
            rewrite_expr(&mut r.condition, spill, misplaced);
        }
        Stmt::If(i) => rewrite_if(i, spill, misplaced),
        Stmt::NumericFor(f) => {
            check_declared(std::slice::from_ref(&f.id), spill, misplaced);
            rewrite_expr(&mut f.start, spill, misplaced);
            rewrite_expr(&mut f.end, spill, misplaced);
            if let Some(step) = &mut f.step {
                rewrite_expr(step, spill, misplaced);
            }
            rewrite_statements(&mut f.statements, spill, misplaced);
        }
        Stmt::For(f) => {
            check_declared(&f.ids, spill, misplaced);
            rewrite_expr(&mut f.expression, spill, misplaced);
            rewrite_statements(&mut f.statements, spill, misplaced);
        }
        Stmt::FunctionDeclaration(f) => {
            if f.localize
                && let Expr::TemporaryIdentifier(t) = &f.name
                && spill.fields.contains_key(&t.id)
            {
                *misplaced = true;
            }
            rewrite_expr(&mut f.name, spill, misplaced);
            check_declared(&f.parameters, spill, misplaced);
            rewrite_statements(&mut f.statements, spill, misplaced);
        }
        Stmt::MethodDeclaration(m) => {
            rewrite_expr(&mut m.expression, spill, misplaced);
            check_declared(&m.parameters, spill, misplaced);
            rewrite_statements(&mut m.statements, spill, misplaced);
        }
        Stmt::VariableDeclaration(v) => {
            check_declared(&v.left, spill, misplaced);
            for e in &mut v.right {
                rewrite_expr(e, spill, misplaced);
            }
        }
        Stmt::Return(r) => {
            for e in &mut r.expressions {
                rewrite_expr(e, spill, misplaced);
            }
        }
    }
}

fn rewrite_if(statement: &mut IfStatement, spill: &Spill, misplaced: &mut bool) {
    rewrite_expr(&mut statement.condition, spill, misplaced);
    rewrite_statements(&mut statement.statements, spill, misplaced);
    match &mut statement.else_body {
        ElseBody::If(next) => rewrite_if(next, spill, misplaced),
        ElseBody::Block(statements) => rewrite_statements(statements, spill, misplaced),
    }
}

fn rewrite_expr(expr: &mut Expr, spill: &Spill, misplaced: &mut bool) {
    match expr {
        Expr::TemporaryIdentifier(t) => {
            if let Some(field) = spill.fields.get(&t.id) {
                *expr = create::property(spill.table.clone(), field.clone());
            }
        }
        Expr::Identifier(_)
        | Expr::None
        | Expr::Nil
        | Expr::False
        | Expr::True
        | Expr::Number(_)
        | Expr::String(_)
        | Expr::LateString(_)
        | Expr::VarArgs => {}
        Expr::ComputedIndex(c) => {
            rewrite_expr(&mut c.expression, spill, misplaced);
            rewrite_expr(&mut c.index, spill, misplaced);
        }
        Expr::PropertyAccess(p) => rewrite_expr(&mut p.expression, spill, misplaced),
        Expr::Call(c) => {
            rewrite_expr(&mut c.expression, spill, misplaced);
            for e in &mut c.args {
                rewrite_expr(e, spill, misplaced);
            }
        }
        Expr::MethodCall(m) => {
            rewrite_expr(&mut m.expression, spill, misplaced);
            for e in &mut m.args {
                rewrite_expr(e, spill, misplaced);
            }
        }
        Expr::Parenthesized(p) => rewrite_expr(&mut p.expression, spill, misplaced),
        Expr::Function(f) => {
            check_declared(&f.parameters, spill, misplaced);
            rewrite_statements(&mut f.statements, spill, misplaced);
        }
        Expr::Binary(b) => {
            rewrite_expr(&mut b.left, spill, misplaced);
            rewrite_expr(&mut b.right, spill, misplaced);
        }
        Expr::Unary(u) => rewrite_expr(&mut u.expression, spill, misplaced),
        Expr::If(i) => {
            rewrite_expr(&mut i.condition, spill, misplaced);
            rewrite_expr(&mut i.expression, spill, misplaced);
            rewrite_expr(&mut i.alternative, spill, misplaced);
        }
        Expr::InterpolatedString(s) => {
            for part in &mut s.parts {
                match part {
                    InterpolatedStringPart::Text(_) => {}
                    InterpolatedStringPart::Expression(e) => rewrite_expr(e, spill, misplaced),
                }
            }
        }
        Expr::Array(items) | Expr::Set(items) => {
            for e in items {
                rewrite_expr(e, spill, misplaced);
            }
        }
        Expr::Map(fields) => {
            for field in fields {
                rewrite_expr(&mut field.index, spill, misplaced);
                rewrite_expr(&mut field.value, spill, misplaced);
            }
        }
        Expr::MixedTable(fields) => {
            for field in fields {
                match field {
                    TableField::Value(e) => rewrite_expr(e, spill, misplaced),
                    TableField::Field(f) => {
                        rewrite_expr(&mut f.index, spill, misplaced);
                        rewrite_expr(&mut f.value, spill, misplaced);
                    }
                }
            }
        }
    }
}
