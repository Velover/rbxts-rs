//! Luau's limits (200 local registers, 255 registers, 200 upvalues per function, its recursion depth), checked on
//! every file a plugin emitted anything into (DESIGN §9: the compiler owns the check for all code, its own output
//! included). Files no plugin emitted into, and builds without plugins, are left as roblox-ts leaves them.
//!
//! Whether a file is within the limits is decided by Luau's own compiler, not predicted: whether a `local` takes a
//! register depends on Luau's constant folding, dead-branch elimination and inlining, which vary with the optimisation
//! and debug levels and between releases. The compiler is [`rbxts_luau_check`]: Luau 0.740 (luau0-src
//! 0.22.0+luau740), pinned exactly in that crate's `Cargo.toml`; a bump can move a file across a limit. Two steps:
//! 1. **Pre-count** (transform time, cheap): a lexical walk counts, per function, every local name live at once
//!    (enclosing blocks and loop variables included), the registers an expression may add on top, the distinct locals
//!    of enclosing functions it uses (upvalues, nested functions' included), and the nesting depth. In the main chunk a
//!    local count past 200 spills the locals plugins hoisted with `hoist_spillable` into one table local (an unneeded
//!    spill is harmless). A file near a limit ([`near_limit`]) is marked for the load check, with the TS statement of
//!    each of its final statements.
//! 2. **Load check** (after rendering): Luau compiles the rendered file, its `--!optimize` directive honoured as in
//!    Roblox ([`decide`]). It fails at optimisation level 1 and debug level 1 (Roblox's default when testing in
//!    Studio), or at level 2 (live games): an error, with Luau's message, at the TS statement that holds the line
//!    Luau names (for a function, the main chunk's statement that contains it). Else it fails at `--!optimize 0` or
//!    debug level 2: a warning, the file loads only with Luau's constant folding. Else it is clean.

use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_luau::{AnyIdentifier, ElseBody, Expr, IfStatement, InterpolatedStringPart, Stmt, TableField, create};
use rbxts_luau_check::{CompileError, LUAU_VERSION};
use rbxts_plugin::PluginDiagnostic;
use tsrs_ast::Diagnostic;
use tsrs_ast::Node;
use tsrs_core::P;

use super::backend::to_diagnostic;
use crate::classes::prereqs::temp_id;
use crate::classes::transform_state::TransformState;

/// The most local registers Luau allows in one function, the main chunk included.
pub const LUAU_MAX_LOCALS: u32 = 200;

/// The pre-count's thresholds for the load check: locals or upvalues (Luau allows 200), locals plus the registers an
/// expression adds (Luau allows 255 in all), and the nesting depth of expressions and blocks (Luau's parser and
/// compiler refuse deep nesting). They leave a margin for what the pre-count does not see.
const NEAR_LOCALS: u32 = 190;
const NEAR_UPVALUES: u32 = 190;
const NEAR_REGISTERS: u32 = 230;
const NEAR_DEPTH: u32 = 100;

/// One function's pre-count (or the main chunk's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    /// The most local names live at once.
    locals: u32,
    /// The most local names live at once plus the registers an expression evaluated there may add.
    registers: u32,
    /// The distinct locals of enclosing functions it uses (0 for the main chunk).
    upvalues: u32,
}

/// The pre-count of a complete file.
#[derive(Debug, Default)]
struct Analysis {
    main: Counts,
    functions: Vec<Counts>,
    /// The deepest nesting of expressions and blocks.
    depth: u32,
}

/// A file may be near one of Luau's limits: Luau's compiler decides.
fn near_limit(analysis: &Analysis) -> bool {
    analysis.depth >= NEAR_DEPTH
        || std::iter::once(&analysis.main).chain(&analysis.functions).any(|counts| {
            counts.locals >= NEAR_LOCALS || counts.upvalues >= NEAR_UPVALUES || counts.registers >= NEAR_REGISTERS
        })
}

/// A local's name in a scope: an identifier's text, or a temporary's id (temporaries never shadow one another, and
/// an identifier never names a temporary).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Name<'a> {
    Text(&'a str),
    Temporary(u32),
}

impl<'a> Name<'a> {
    fn of(id: &'a AnyIdentifier) -> Name<'a> {
        match id {
            AnyIdentifier::Identifier(id) => Name::Text(&id.name),
            AnyIdentifier::Temporary(id) => Name::Temporary(id.id),
        }
    }

    /// The name an identifier expression uses.
    fn used(e: &'a Expr) -> Option<Name<'a>> {
        match e {
            Expr::Identifier(id) => Some(Name::Text(&id.name)),
            Expr::TemporaryIdentifier(id) => Some(Name::Temporary(id.id)),
            _ => None,
        }
    }
}

/// One function being walked.
#[derive(Default)]
struct Frame {
    /// The bindings of enclosing functions it uses.
    upvalues: FxHashSet<usize>,
    counts: Counts,
}

/// Walks a file in lexical order with its scopes, resolving every use of a local to its binding.
#[derive(Default)]
struct Walker<'a> {
    /// Per binding: the depth of the function that declares it (0: the main chunk).
    bindings: Vec<usize>,
    /// The locals in scope, innermost last.
    scopes: Vec<FxHashMap<Name<'a>, usize>>,
    /// The functions being walked, outermost (the main chunk) first.
    frames: Vec<Frame>,
    functions: Vec<Counts>,
    /// The current and deepest nesting.
    depth: u32,
    deepest: u32,
}

impl<'a> Walker<'a> {
    fn lookup(&self, name: Name<'a>) -> Option<usize> {
        self.scopes.iter().rev().find_map(|scope| scope.get(&name).copied())
    }

    fn bind(&mut self, name: Name<'a>) {
        let index = self.bindings.len();
        self.bindings.push(self.frames.len() - 1);
        self.scopes.last_mut().expect("a scope").insert(name, index);
    }

    /// A use of a name: an upvalue of every function between its declaration and here.
    fn use_name(&mut self, name: Name<'a>) {
        if let Some(index) = self.lookup(name) {
            let depth = self.bindings[index];
            for frame in &mut self.frames[depth + 1..] {
                frame.upvalues.insert(index);
            }
        }
    }

    /// Notes `active` live locals plus `extra` registers in the current function.
    fn pressure(&mut self, active: u32, extra: u32) {
        let counts = &mut self.frames.last_mut().expect("a frame").counts;
        counts.locals = counts.locals.max(active);
        counts.registers = counts.registers.max(active + extra);
    }

    fn nest(&mut self) {
        self.depth += 1;
        self.deepest = self.deepest.max(self.depth);
    }

    /// Walks the main chunk.
    fn run(mut self, statements: &'a [Stmt]) -> Analysis {
        self.frames.push(Frame::default());
        self.scopes.push(FxHashMap::default());
        self.top(statements, 0);
        let main = self.frames.pop().expect("the main chunk's frame").counts;
        Analysis { main, functions: self.functions, depth: self.deepest }
    }

    /// A function: its parameters (and `self`) and body in a new frame. Returns the registers its closure takes.
    fn function(&mut self, parameters: &'a [AnyIdentifier], implicit_self: bool, body: &'a [Stmt]) -> u32 {
        self.nest();
        self.frames.push(Frame::default());
        self.scopes.push(FxHashMap::default());
        if implicit_self {
            self.bind(Name::Text("self"));
        }
        for id in parameters {
            self.bind(Name::of(id));
        }
        let entry = (parameters.len() + usize::from(implicit_self)) as u32;
        self.pressure(entry, 0);
        self.top(body, entry);
        self.scopes.pop();
        let frame = self.frames.pop().expect("the function's frame");
        self.functions.push(Counts { upvalues: frame.upvalues.len() as u32, ..frame.counts });
        self.depth -= 1;
        1
    }

    /// The uses and functions in an expression, and the registers evaluating it may take (an estimate: one per
    /// value held at once).
    fn expr(&mut self, e: &'a Expr) -> u32 {
        self.nest();
        let registers = match e {
            Expr::Identifier(_) | Expr::TemporaryIdentifier(_) => {
                if let Some(name) = Name::used(e) {
                    self.use_name(name);
                }
                1
            }
            Expr::Function(f) => self.function(&f.parameters, false, &f.statements),
            Expr::ComputedIndex(c) => {
                let object = self.expr(&c.expression);
                let index = self.expr(&c.index);
                object.max(1 + index)
            }
            Expr::PropertyAccess(p) => self.expr(&p.expression),
            Expr::Call(c) => {
                let callee = self.expr(&c.expression);
                callee.max(self.list(&c.args, 1))
            }
            Expr::MethodCall(m) => {
                let object = self.expr(&m.expression);
                object.max(self.list(&m.args, 2))
            }
            Expr::Parenthesized(p) => self.expr(&p.expression),
            Expr::Binary(b) => {
                let left = self.expr(&b.left);
                let right = self.expr(&b.right);
                left.max(1 + right)
            }
            Expr::Unary(u) => self.expr(&u.expression),
            Expr::If(i) => {
                let condition = self.expr(&i.condition);
                let then = self.expr(&i.expression);
                let otherwise = self.expr(&i.alternative);
                condition.max(then).max(otherwise)
            }
            Expr::InterpolatedString(s) => {
                let mut most = 1;
                let mut held = 1;
                for part in &s.parts {
                    if let InterpolatedStringPart::Expression(e) = part {
                        most = most.max(held + self.expr(e));
                        held += 1;
                    }
                }
                most
            }
            Expr::Array(items) | Expr::Set(items) => self.list(items, 1),
            Expr::Map(fields) => {
                let mut most = 1;
                for field in fields {
                    let index = self.expr(&field.index);
                    let value = self.expr(&field.value);
                    most = most.max(1 + index.max(1 + value));
                }
                most
            }
            Expr::MixedTable(fields) => {
                let mut most = 1;
                let mut held = 1;
                for field in fields {
                    match field {
                        TableField::Value(e) => {
                            most = most.max(held + self.expr(e));
                            held += 1;
                        }
                        TableField::Field(f) => {
                            let index = self.expr(&f.index);
                            let value = self.expr(&f.value);
                            most = most.max(held + index.max(1 + value));
                        }
                    }
                }
                most
            }
            Expr::None
            | Expr::Nil
            | Expr::False
            | Expr::True
            | Expr::Number(_)
            | Expr::String(_)
            | Expr::LateString(_)
            | Expr::VarArgs => 1,
        };
        self.depth -= 1;
        registers
    }

    /// Values evaluated into consecutive registers after `held` others (call arguments, table items, a statement's
    /// values): the most registers at once.
    fn list(&mut self, values: &'a [Expr], held: u32) -> u32 {
        let mut most = held;
        for (i, value) in values.iter().enumerate() {
            most = most.max(held + i as u32 + self.expr(value));
        }
        most
    }

    /// An assignment target, or the name of `function name()`.
    fn target(&mut self, e: &'a Expr) -> u32 {
        match Name::used(e) {
            Some(name) => {
                self.use_name(name);
                0
            }
            None => self.expr(e),
        }
    }

    /// What a statement declares in its block (binding the names), on top of `active` live locals.
    fn statement(&mut self, statement: &'a Stmt, active: u32) -> u32 {
        match statement {
            Stmt::Assignment(a) => {
                let values = self.list(&a.right, 0);
                let targets = a.left.iter().map(|target| self.target(target)).max().unwrap_or(0);
                self.pressure(active, values + targets);
                0
            }
            Stmt::Break | Stmt::Continue | Stmt::Comment(_) => 0,
            Stmt::Call(e) => {
                let registers = self.expr(e);
                self.pressure(active, registers);
                0
            }
            Stmt::Do(block) => {
                self.block(&block.statements, active);
                0
            }
            Stmt::While(w) => {
                let registers = self.expr(&w.condition);
                self.pressure(active, registers);
                self.block(&w.statements, active);
                0
            }
            Stmt::Repeat(r) => {
                // the condition sees the block's locals
                self.nest();
                self.scopes.push(FxHashMap::default());
                let inner = self.top(&r.statements, active);
                let registers = self.expr(&r.condition);
                self.pressure(inner, registers);
                self.scopes.pop();
                self.depth -= 1;
                0
            }
            Stmt::If(i) => {
                self.if_statement(i, active);
                0
            }
            Stmt::NumericFor(f) => {
                let mut registers = self.expr(&f.start).max(1 + self.expr(&f.end));
                if let Some(step) = &f.step {
                    registers = registers.max(2 + self.expr(step));
                }
                // the loop's three internal registers, then its variable
                self.pressure(active, registers.max(3));
                self.scopes.push(FxHashMap::default());
                self.bind(Name::of(&f.id));
                self.block(&f.statements, active + 4);
                self.scopes.pop();
                0
            }
            Stmt::For(f) => {
                let registers = self.expr(&f.expression);
                self.pressure(active, registers.max(3));
                self.scopes.push(FxHashMap::default());
                for id in &f.ids {
                    self.bind(Name::of(id));
                }
                self.block(&f.statements, active + 3 + f.ids.len() as u32);
                self.scopes.pop();
                0
            }
            Stmt::FunctionDeclaration(f) => {
                // `local function f` is in scope in its own body; `function f()` writes `f`
                let declared = match (f.localize, Name::used(&f.name)) {
                    (true, Some(name)) => {
                        self.bind(name);
                        1
                    }
                    _ => {
                        self.target(&f.name);
                        0
                    }
                };
                self.function(&f.parameters, false, &f.statements);
                self.pressure(active + declared, 1);
                declared
            }
            Stmt::MethodDeclaration(m) => {
                let object = self.expr(&m.expression);
                self.function(&m.parameters, true, &m.statements);
                self.pressure(active, object + 1);
                0
            }
            Stmt::VariableDeclaration(declaration) => {
                let values = self.list(&declaration.right, 0);
                for id in &declaration.left {
                    self.bind(Name::of(id));
                }
                let declared = declaration.left.len() as u32;
                self.pressure(active, values.max(declared));
                declared
            }
            Stmt::Return(r) => {
                let registers = self.list(&r.expressions, 0);
                self.pressure(active, registers);
                0
            }
        }
    }

    fn if_statement(&mut self, statement: &'a IfStatement, active: u32) {
        let registers = self.expr(&statement.condition);
        self.pressure(active, registers);
        self.block(&statement.statements, active);
        match &statement.else_body {
            ElseBody::If(next) => self.if_statement(next, active),
            ElseBody::Block(statements) => self.block(statements, active),
        }
    }

    /// A nested block on top of `active` live locals.
    fn block(&mut self, statements: &'a [Stmt], active: u32) {
        self.nest();
        self.scopes.push(FxHashMap::default());
        self.top(statements, active);
        self.scopes.pop();
        self.depth -= 1;
    }

    /// A block's statements in the current scope, from `start` live locals; returns the locals live at its end.
    fn top(&mut self, statements: &'a [Stmt], start: u32) -> u32 {
        let mut active = start;
        for statement in statements {
            active += self.statement(statement, active);
            self.pressure(active, 0);
        }
        active
    }
}

/// The pre-count of a complete file.
fn analyse(statements: &[Stmt]) -> Analysis {
    Walker::default().run(statements)
}

/// The optimisation level a `--!optimize N` directive among the file's leading comments sets, if any.
fn optimize_directive(statements: &[Stmt]) -> Option<u32> {
    statements
        .iter()
        .map_while(|s| match s {
            Stmt::Comment(text) => Some(text),
            _ => None,
        })
        .find_map(|text| text.strip_prefix("!optimize").map(|level| level.trim().parse().unwrap_or(1)))
}

/// A file marked for the load check: the TS node of each of its final (main-chunk) statements, and its
/// `--!optimize` directive.
pub(crate) struct LoadCheck {
    anchors: Vec<P<Node>>,
    directive: Option<u32>,
}

/// Checks a complete file a plugin emitted anything into (`plugin_file.emitted`): spills the `hoist_spillable`
/// locals when the main chunk may pass Luau's local limit, and marks the file for the load check when it is near
/// one of Luau's limits (`plugin_file.load_check`). Files no plugin emitted into are left as roblox-ts leaves them.
pub(crate) fn check_main_chunk(state: &mut TransformState<'_>, statements: &mut Vec<Stmt>) {
    if !state.plugin_file.emitted {
        return;
    }
    let analysis = analyse(statements);
    if !near_limit(&analysis) {
        return;
    }
    let mut spilled_at = None;
    if analysis.main.locals > LUAU_MAX_LOCALS && !state.plugin_file.spillable.is_empty() {
        spilled_at = spill(state, statements);
    }
    let file = state.source_file.as_node();
    let anchors = (0..statements.len()).map(|i| statement_node(state, i, spilled_at).unwrap_or(file)).collect();
    state.plugin_file.load_check = Some(LoadCheck { anchors, directive: optimize_directive(statements) });
}

/// Where Luau refused a file.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Refusal {
    error: CompileError,
    optimization_level: u8,
    debug_level: u8,
}

/// What Luau's compiler says about a rendered file.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Verdict {
    Loads,
    /// It loads at optimisation levels 1 and 2 at debug level 1, but not at `--!optimize 0` or debug level 2: only
    /// with Luau's constant folding.
    FoldingOnly(Refusal),
    /// It fails at optimisation level 1 or 2 at debug level 1: in Studio or in live games.
    Fails(Refusal),
}

/// Compiles `source` at the levels Roblox uses first, then at the ones that fold nothing.
fn decide(source: &str) -> Verdict {
    let at = |optimization_level, debug_level| {
        rbxts_luau_check::compile(source, optimization_level, debug_level).err().map(|error| Refusal {
            error,
            optimization_level,
            debug_level,
        })
    };
    if let Some(refusal) = at(1, 1).or_else(|| at(2, 1)) {
        return Verdict::Fails(refusal);
    }
    match at(0, 1).or_else(|| at(1, 2)) {
        Some(refusal) => Verdict::FoldingOnly(refusal),
        None => Verdict::Loads,
    }
}

/// The load check of a rendered file marked by [`check_main_chunk`]: an error when Luau refuses it where Roblox runs
/// it, a warning when it loads only with Luau's constant folding. `lines` is the line each final statement starts on.
pub(crate) fn load_check(check: &LoadCheck, luau: &str, lines: &[u32]) -> Option<P<Diagnostic>> {
    let verdict = decide(luau);
    let (Verdict::Fails(refusal) | Verdict::FoldingOnly(refusal)) = &verdict else { return None };
    let index = lines.partition_point(|&line| line <= refusal.error.line).saturating_sub(1);
    let anchor = check.anchors.get(index).or(check.anchors.first())?;
    let luau_says = format!("\"{}\" (output line {})", refusal.error.message, refusal.error.line);
    let level = |optimization_level: u8| match check.directive {
        Some(directive) => format!("optimisation level {directive} (the file's --!optimize directive)"),
        None => format!("optimisation level {optimization_level}"),
    };
    let diagnostic = match &verdict {
        Verdict::Fails(refusal) => {
            let r#where = if refusal.optimization_level == 1 {
                "Roblox's default when testing in Studio"
            } else {
                "what live games run"
            };
            PluginDiagnostic::error(
                *anchor,
                format!(
                    "Luau {LUAU_VERSION} refuses this file at {} and debug level 1, {where}: {luau_says}. It would \
                     build, then fail to load.",
                    level(refusal.optimization_level)
                ),
            )
        }
        _ => PluginDiagnostic::warning(
            *anchor,
            format!(
                "This file loads only with Luau's constant folding: Luau {LUAU_VERSION} refuses it at {} and debug \
                 level {}: {luau_says}. It fails to load at --!optimize 0 or debug level 2.",
                level(refusal.optimization_level),
                refusal.debug_level
            ),
        ),
    };
    Some(to_diagnostic(&diagnostic))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn f() -> Expr {
        create::call(create::id("f"), vec![])
    }

    /// `local <prefix>0 = value` … `local <prefix><count - 1> = value`
    fn locals(prefix: &str, count: usize, value: &dyn Fn() -> Expr) -> Vec<Stmt> {
        (0..count).map(|i| create::local(create::any_id(format!("{prefix}{i}")), Some(value()))).collect()
    }

    fn uses(prefix: &str, count: usize) -> Vec<Expr> {
        (0..count).map(|i| create::id(format!("{prefix}{i}"))).collect()
    }

    #[test]
    fn the_pre_count_counts_every_name() {
        // constants count: whether Luau folds them is the load check's question
        let analysis = analyse(&locals("x", 195, &|| create::number(1.0)));
        assert_eq!(analysis.main.locals, 195);
        assert!(near_limit(&analysis));
        let analysis = analyse(&locals("x", 150, &f));
        assert!(!near_limit(&analysis), "{analysis:?}");
        // nested blocks and loop variables add to the enclosing locals
        let mut statements = locals("x", 150, &f);
        let body = locals("y", 30, &f);
        statements.push(create::numeric_for(create::any_id("i"), create::number(1.0), create::number(3.0), None, body));
        assert_eq!(analyse(&statements).main.locals, 150 + 4 + 30);
        // a call with many arguments needs registers past the locals
        let mut statements = locals("x", 150, &f);
        statements.push(create::call_statement(create::call(create::id("print"), uses("x", 100))));
        let analysis = analyse(&statements);
        assert_eq!(analysis.main.locals, 150);
        assert!(analysis.main.registers >= 250 && near_limit(&analysis), "{analysis:?}");
        // deep nesting
        let deep = (0..120).fold(create::number(0.0), |e, i| {
            create::binary(e, rbxts_luau::BinaryOperator::Add, create::number(f64::from(i)))
        });
        let analysis = analyse(&[create::local(create::any_id("x"), Some(deep))]);
        assert!(analysis.depth > 120 && near_limit(&analysis), "{analysis:?}");
    }

    #[test]
    fn upvalues_are_the_enclosing_locals_a_function_and_its_nested_functions_use() {
        // the main chunk declares a0…a149; `o` declares b0…b59 and returns a function that uses all of them
        let mut statements = locals("a", 150, &f);
        let mut body = locals("b", 60, &f);
        let mut used = uses("a", 150);
        used.extend(uses("b", 60));
        let inner =
            create::function(vec![], false, vec![create::call_statement(create::call(create::id("print"), used))]);
        body.push(create::return_stmt(inner));
        statements.push(create::function_declaration(true, create::id("o"), vec![], false, body));
        let analysis = analyse(&statements);
        let upvalues: Vec<u32> = analysis.functions.iter().map(|f| f.upvalues).collect();
        assert_eq!(upvalues, vec![210, 150]);
        assert!(near_limit(&analysis));
    }

    /// `count` lines of `local <name><i> = <value>`.
    fn lua(count: usize, value: &str) -> String {
        (0..count).map(|i| format!("local x{i} = {value}\n")).collect()
    }

    fn refused_at(verdict: &Verdict) -> Option<(u8, u8, &str)> {
        match verdict {
            Verdict::Loads => None,
            Verdict::Fails(r) | Verdict::FoldingOnly(r) => {
                Some((r.optimization_level, r.debug_level, r.error.message.as_str()))
            }
        }
    }

    #[test]
    fn luau_decides() {
        assert_eq!(decide(&lua(200, "f()")), Verdict::Loads);
        // 201 calls: refused wherever it runs
        let verdict = decide(&lua(201, "f()"));
        assert!(matches!(verdict, Verdict::Fails(_)), "{verdict:?}");
        let (o, d, message) = refused_at(&verdict).unwrap();
        assert_eq!((o, d), (1, 1));
        assert!(message.starts_with("Out of local registers"), "{message}");
        // 201 constants: folded at optimisation level 1+ and debug level 1 only
        let verdict = decide(&lua(201, "1"));
        assert!(matches!(verdict, Verdict::FoldingOnly(_)), "{verdict:?}");
        assert_eq!(refused_at(&verdict).map(|(o, d, _)| (o, d)), Some((0, 1)));
        // what the old emulation got wrong: string comparisons (never folded), dead branches (skipped from level 1)
        assert!(matches!(decide(&lua(201, "\"a\" == \"a\"")), Verdict::FoldingOnly(_) | Verdict::Fails(_)));
        let dead = format!("{}if false then\n{}end\n", lua(195, "f()"), lua(10, "f()"));
        assert!(matches!(decide(&dead), Verdict::FoldingOnly(_)), "{:?}", decide(&dead));
        // the file's directive wins
        assert!(matches!(decide(&format!("--!optimize 0\n{}", lua(201, "1"))), Verdict::Fails(_)));
        // upvalues and recursion depth are Luau's to judge too
        let upvalues = format!("{}local g = function() print({}) end\n", lua(201, "f()"), {
            (0..201).map(|i| format!("x{i}")).collect::<Vec<_>>().join(", ")
        });
        let verdict = decide(&upvalues);
        assert!(matches!(verdict, Verdict::Fails(_)), "{verdict:?}");
        assert!(refused_at(&verdict).unwrap().2.contains("upvalue"), "{verdict:?}");
        let sum = (0..2000).map(|i| i.to_string()).collect::<Vec<_>>().join(" + ");
        let verdict = decide(&format!("local s = {sum}\n"));
        assert!(matches!(verdict, Verdict::Fails(_)), "{verdict:?}");
    }
}
