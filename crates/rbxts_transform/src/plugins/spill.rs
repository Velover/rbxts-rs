//! Luau's limit of 200 local registers in a function, checked on the main chunk of a file a plugin added locals to,
//! once the file is complete (DESIGN §9: the compiler owns the check for all code, its own output included).
//!
//! Luau gives a register to every local that is live, the locals of enclosing blocks included, except that from
//! optimisation level 1 (Roblox's default; scripts opt into 2 with `--!optimize 2`) a local initialised with a
//! constant and never assigned is folded into a constant and takes none. So each local is counted as one that
//! surely takes a register (a call, a table or a function is its value, or it is assigned somewhere), one that
//! surely does not (a literal or nothing, never assigned), or one that may (anything else, which Luau may fold). At
//! `--!optimize 0` every local takes a register.
//!
//! When the main chunk may pass the limit, the locals plugins hoisted with `hoist_spillable` move into one table
//! local first (an unneeded spill is harmless). A chunk that surely still passes it is an error, as it would build
//! and then fail to load ("Out of local registers"); one that may pass it is a warning.

use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_luau::{
    AnyIdentifier, ElseBody, Expr, IfStatement, InterpolatedStringPart, Stmt, TableField, UnaryOperator,
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

/// Register counts: those surely taken at optimisation level 1, those that may be (folding decides), and every
/// local (level 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Registers {
    sure: u32,
    maybe: u32,
    all: u32,
}

impl Registers {
    fn add(self, other: Registers) -> Registers {
        Registers { sure: self.sure + other.sure, maybe: self.maybe + other.maybe, all: self.all + other.all }
    }

    fn max(self, other: Registers) -> Registers {
        Registers { sure: self.sure.max(other.sure), maybe: self.maybe.max(other.maybe), all: self.all.max(other.all) }
    }

    const SURE: Registers = Registers { sure: 1, maybe: 1, all: 1 };
    const MAYBE: Registers = Registers { sure: 0, maybe: 1, all: 1 };
    const FOLDED: Registers = Registers { sure: 0, maybe: 0, all: 1 };
}

/// The names and temporaries assigned anywhere in the file (a local assigned somewhere is never folded), and the
/// optimisation level.
#[derive(Default)]
struct Assigned {
    /// The optimisation level the file is compiled at.
    level: u32,
    names: FxHashSet<String>,
    temporaries: FxHashSet<u32>,
}

impl Assigned {
    fn contains(&self, id: &AnyIdentifier) -> bool {
        match id {
            AnyIdentifier::Identifier(id) => self.names.contains(&id.name),
            AnyIdentifier::Temporary(id) => self.temporaries.contains(&id.id),
        }
    }
}

/// What one `local` declaration's names take.
fn declaration_registers(declaration: &VariableDeclaration, assigned: &Assigned) -> Registers {
    let mut total = Registers::default();
    let spreads = declaration.right.last().is_some_and(|last| matches!(last, Expr::Call(_) | Expr::MethodCall(_)));
    for (i, id) in declaration.left.iter().enumerate() {
        let value = declaration.right.get(i);
        let kind = if assigned.contains(id) {
            Registers::SURE
        } else {
            match value {
                // `local a, b = f()`: `b` takes f's second value
                None if spreads => Registers::SURE,
                // nothing (nil), never assigned
                None if declaration.right.is_empty() => Registers::FOLDED,
                None => Registers::MAYBE,
                Some(
                    Expr::Nil | Expr::True | Expr::False | Expr::Number(_) | Expr::String(_) | Expr::LateString(_),
                ) => Registers::FOLDED,
                Some(Expr::Unary(unary))
                    if unary.operator == UnaryOperator::Neg && matches!(unary.expression, Expr::Number(_)) =>
                {
                    Registers::FOLDED
                }
                // at -O2 Luau may fold a builtin call with constant arguments
                Some(Expr::Call(call)) if assigned.level >= 2 && call.args.iter().all(Expr::is_simple_primitive) => {
                    Registers::MAYBE
                }
                // an index is never folded, but for a builtin constant (`math.pi`) of a global
                Some(Expr::PropertyAccess(access)) if !matches!(access.expression, Expr::Identifier(_)) => {
                    Registers::SURE
                }
                Some(Expr::ComputedIndex(access)) if !matches!(access.expression, Expr::Identifier(_)) => {
                    Registers::SURE
                }
                Some(
                    Expr::Call(_)
                    | Expr::MethodCall(_)
                    | Expr::Function(_)
                    | Expr::Array(_)
                    | Expr::Map(_)
                    | Expr::Set(_)
                    | Expr::MixedTable(_)
                    | Expr::VarArgs,
                ) => Registers::SURE,
                Some(_) => Registers::MAYBE,
            }
        };
        total = total.add(kind);
    }
    total
}

/// What a statement declares in the block it is in.
fn statement_registers(statement: &Stmt, assigned: &Assigned) -> Registers {
    match statement {
        Stmt::VariableDeclaration(declaration) => declaration_registers(declaration, assigned),
        Stmt::FunctionDeclaration(declaration) if declaration.localize => Registers::SURE,
        _ => Registers::default(),
    }
}

/// The most registers live at once inside a statement's nested blocks, on top of `active` (function bodies are
/// functions of their own).
fn nested_pressure(statement: &Stmt, active: Registers, assigned: &Assigned) -> Registers {
    match statement {
        Stmt::Do(block) => block_pressure(&block.statements, active, assigned),
        Stmt::While(w) => block_pressure(&w.statements, active, assigned),
        Stmt::Repeat(r) => block_pressure(&r.statements, active, assigned),
        Stmt::If(i) => if_pressure(i, active, assigned),
        Stmt::NumericFor(f) => block_pressure(&f.statements, active.add(Registers::SURE), assigned),
        Stmt::For(f) => {
            let ids = Registers { sure: f.ids.len() as u32, maybe: f.ids.len() as u32, all: f.ids.len() as u32 };
            block_pressure(&f.statements, active.add(ids), assigned)
        }
        _ => active,
    }
}

fn if_pressure(statement: &IfStatement, active: Registers, assigned: &Assigned) -> Registers {
    let then = block_pressure(&statement.statements, active, assigned);
    match &statement.else_body {
        ElseBody::If(next) => then.max(if_pressure(next, active, assigned)),
        ElseBody::Block(statements) => then.max(block_pressure(statements, active, assigned)),
    }
}

fn block_pressure(statements: &[Stmt], mut active: Registers, assigned: &Assigned) -> Registers {
    let mut most = active;
    for statement in statements {
        most = most.max(nested_pressure(statement, active, assigned));
        active = active.add(statement_registers(statement, assigned));
        most = most.max(active);
    }
    most
}

/// The main chunk's pressure, and for each count the index of the top-level statement where it first passes the
/// limit.
fn chunk_pressure(statements: &[Stmt], assigned: &Assigned) -> (Registers, [Option<usize>; 3]) {
    let mut active = Registers::default();
    let mut most = active;
    let mut passed = [None; 3];
    for (i, statement) in statements.iter().enumerate() {
        let inner = nested_pressure(statement, active, assigned);
        active = active.add(statement_registers(statement, assigned));
        let here = inner.max(active);
        for (slot, count) in passed.iter_mut().zip([here.sure, here.maybe, here.all]) {
            if slot.is_none() && count > LUAU_MAX_LOCALS {
                *slot = Some(i);
            }
        }
        most = most.max(here);
    }
    (most, passed)
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

/// Checks the complete main chunk of a file a plugin added locals to (`plugin_file.added_locals`): spills the
/// `hoist_spillable` locals into one table when it may pass Luau's limit, then reports an error if it surely still
/// does (a warning if it may). Files no plugin added a local to are left as roblox-ts leaves them.
pub(crate) fn check_main_chunk(state: &mut TransformState<'_>, statements: &mut Vec<Stmt>) {
    if !state.plugin_file.added_locals {
        return;
    }
    let level = optimize_level(statements);
    let mut assigned = Assigned { level, ..Assigned::default() };
    collect_assigned(statements, &mut assigned);
    let (mut most, mut passed) = chunk_pressure(statements, &assigned);
    let may_pass = if level == 0 { most.all } else { most.maybe } > LUAU_MAX_LOCALS;
    if !may_pass {
        return;
    }
    let mut spilled_at = None;
    if !state.plugin_file.spillable.is_empty() {
        spilled_at = spill(state, statements);
        (most, passed) = chunk_pressure(statements, &assigned);
    }
    let (count, at, sure) = match level {
        0 => (most.all, passed[2], true),
        _ if most.sure > LUAU_MAX_LOCALS => (most.sure, passed[0], true),
        _ => (most.maybe, passed[1], false),
    };
    if count <= LUAU_MAX_LOCALS {
        return;
    }
    let anchor = at.and_then(|at| statement_node(state, at, spilled_at)).unwrap_or_else(|| state.source_file.as_node());
    let diagnostic = if sure {
        let why = if level == 0 { ", at --!optimize 0 (every local takes one)" } else { "" };
        PluginDiagnostic::error(
            anchor,
            format!(
                "This file needs {count} local registers in its main chunk{why}, and Luau allows at most \
                 {LUAU_MAX_LOCALS}: it would build, then fail to load (\"Out of local registers\"). The count passes \
                 the limit here."
            ),
        )
    } else {
        PluginDiagnostic::warning(
            anchor,
            format!(
                "This file may need up to {count} local registers in its main chunk ({} for certain; Luau folds \
                 constant locals into no register), and Luau allows at most {LUAU_MAX_LOCALS}: it may fail to load \
                 (\"Out of local registers\"). The count passes the limit here.",
                most.sure
            ),
        )
    };
    state.add_diagnostic(to_diagnostic(&diagnostic));
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

fn collect_assigned(statements: &[Stmt], out: &mut Assigned) {
    for statement in statements {
        collect_assigned_statement(statement, out);
    }
}

fn note_assigned(target: &Expr, out: &mut Assigned) {
    match target {
        Expr::Identifier(id) => {
            out.names.insert(id.name.clone());
        }
        Expr::TemporaryIdentifier(id) => {
            out.temporaries.insert(id.id);
        }
        _ => {}
    }
}

fn collect_assigned_statement(statement: &Stmt, out: &mut Assigned) {
    match statement {
        Stmt::Assignment(assignment) => {
            for target in &assignment.left {
                note_assigned(target, out);
            }
            for e in assignment.left.iter().chain(&assignment.right) {
                collect_assigned_expr(e, out);
            }
        }
        Stmt::Break | Stmt::Continue | Stmt::Comment(_) => {}
        Stmt::Call(e) => collect_assigned_expr(e, out),
        Stmt::Do(block) => collect_assigned(&block.statements, out),
        Stmt::While(w) => {
            collect_assigned_expr(&w.condition, out);
            collect_assigned(&w.statements, out);
        }
        Stmt::Repeat(r) => {
            collect_assigned(&r.statements, out);
            collect_assigned_expr(&r.condition, out);
        }
        Stmt::If(i) => collect_assigned_if(i, out),
        Stmt::NumericFor(f) => {
            collect_assigned_expr(&f.start, out);
            collect_assigned_expr(&f.end, out);
            if let Some(step) = &f.step {
                collect_assigned_expr(step, out);
            }
            collect_assigned(&f.statements, out);
        }
        Stmt::For(f) => {
            collect_assigned_expr(&f.expression, out);
            collect_assigned(&f.statements, out);
        }
        Stmt::FunctionDeclaration(f) => {
            // `function name()` assigns `name`
            if !f.localize {
                note_assigned(&f.name, out);
            }
            collect_assigned_expr(&f.name, out);
            collect_assigned(&f.statements, out);
        }
        Stmt::MethodDeclaration(m) => {
            collect_assigned_expr(&m.expression, out);
            collect_assigned(&m.statements, out);
        }
        Stmt::VariableDeclaration(v) => {
            for e in &v.right {
                collect_assigned_expr(e, out);
            }
        }
        Stmt::Return(r) => {
            for e in &r.expressions {
                collect_assigned_expr(e, out);
            }
        }
    }
}

fn collect_assigned_if(statement: &IfStatement, out: &mut Assigned) {
    collect_assigned_expr(&statement.condition, out);
    collect_assigned(&statement.statements, out);
    match &statement.else_body {
        ElseBody::If(next) => collect_assigned_if(next, out),
        ElseBody::Block(statements) => collect_assigned(statements, out),
    }
}

fn collect_assigned_expr(expr: &Expr, out: &mut Assigned) {
    match expr {
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
        Expr::ComputedIndex(c) => {
            collect_assigned_expr(&c.expression, out);
            collect_assigned_expr(&c.index, out);
        }
        Expr::PropertyAccess(p) => collect_assigned_expr(&p.expression, out),
        Expr::Call(c) => {
            collect_assigned_expr(&c.expression, out);
            for e in &c.args {
                collect_assigned_expr(e, out);
            }
        }
        Expr::MethodCall(m) => {
            collect_assigned_expr(&m.expression, out);
            for e in &m.args {
                collect_assigned_expr(e, out);
            }
        }
        Expr::Parenthesized(p) => collect_assigned_expr(&p.expression, out),
        Expr::Function(f) => collect_assigned(&f.statements, out),
        Expr::Binary(b) => {
            collect_assigned_expr(&b.left, out);
            collect_assigned_expr(&b.right, out);
        }
        Expr::Unary(u) => collect_assigned_expr(&u.expression, out),
        Expr::If(i) => {
            collect_assigned_expr(&i.condition, out);
            collect_assigned_expr(&i.expression, out);
            collect_assigned_expr(&i.alternative, out);
        }
        Expr::InterpolatedString(s) => {
            for part in &s.parts {
                if let InterpolatedStringPart::Expression(e) = part {
                    collect_assigned_expr(e, out);
                }
            }
        }
        Expr::Array(items) | Expr::Set(items) => {
            for e in items {
                collect_assigned_expr(e, out);
            }
        }
        Expr::Map(fields) => {
            for field in fields {
                collect_assigned_expr(&field.index, out);
                collect_assigned_expr(&field.value, out);
            }
        }
        Expr::MixedTable(fields) => {
            for field in fields {
                match field {
                    TableField::Value(e) => collect_assigned_expr(e, out),
                    TableField::Field(f) => {
                        collect_assigned_expr(&f.index, out);
                        collect_assigned_expr(&f.value, out);
                    }
                }
            }
        }
    }
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
