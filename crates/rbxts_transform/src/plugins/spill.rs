//! Luau's limits of 200 local registers and 200 upvalues per function, checked on every file a plugin emitted
//! anything into, once the file is complete (DESIGN §9: the compiler owns the check for all code, its own output
//! included).
//!
//! **Locals.** Luau gives a register to every live local: those of enclosing blocks count while a nested block runs,
//! a numeric `for` adds its loop variable and a generic `for` its names. Whether a `local` statement takes registers
//! depends on how the script is compiled: from optimisation level 1, and only at debug level 1, Luau folds a
//! statement into no register when every name it declares is a constant, which needs no more values than names, no
//! name written anywhere (an assignment, a compound assignment or `function name()`, to that binding, from any
//! function), and each value folding into a constant at optimisation level 1: a literal, an interpolated string
//! whose parts fold to strings, an operator on such constants (`-1`, `1 + 2`, `"a" .. "b"`, `not x`, `#"s"`,
//! comparisons, `and` / `or` / if-expressions that pick a constant), or a use of a constant local (a missing value is
//! `nil`, unless the last value is a call or `...`). Builtin calls and reads (`math.abs(1)`, `math.pi`) fold only at
//! level 2, and loop variables always take a register at level 1 (only level 2 unrolls loops). So two counts are
//! kept, per function and at the deepest point:
//! - **certain**: the locals that take a register at optimisation level 1 and debug level 1, Roblox's default when
//!   testing in Studio and the setting where Luau folds the most at level 1 (at `--!optimize 0` every local);
//! - **conservative**: every local name (what takes a register at `--!optimize 0` or debug level 2).
//!
//! In the main chunk, a conservative count past the limit spills the locals plugins hoisted with `hoist_spillable`
//! into one table local (an unneeded spill is harmless). A certain count still past the limit is an error: the file
//! fails to load at optimisation level 1 and debug level 1 ("Out of local registers"), and so in Studio. A
//! conservative count past it is a warning: the file loads only where Luau folds constants (optimisation level 1 or
//! higher at debug level 1) and fails at `--!optimize 0` or debug level 2. Every function of the file gets the same
//! check (no spill); its diagnostic is anchored at the main chunk's statement that contains it.
//!
//! **Upvalues.** A function's upvalues are the distinct locals of enclosing functions it uses, those its nested
//! functions use from outside it included. Past 200 the count is a warning naming the function: Luau folds the uses
//! of constant locals instead of capturing them (optimisation level 1 or higher at debug level 1), so the function
//! may still load there.

use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_luau::{
    AnyIdentifier, BinaryOperator, ElseBody, Expr, IfStatement, InterpolatedStringPart, Stmt, TableField,
    UnaryOperator, VariableDeclaration, create,
};
use rbxts_plugin::PluginDiagnostic;
use tsrs_ast::Node;
use tsrs_core::P;

use super::backend::to_diagnostic;
use crate::classes::prereqs::temp_id;
use crate::classes::transform_state::TransformState;

/// The most local registers Luau allows in one function, the main chunk included.
pub const LUAU_MAX_LOCALS: u32 = 200;

/// The most upvalues Luau allows in one function.
pub const LUAU_MAX_UPVALUES: u32 = 200;

/// Register counts: every local name (`conservative`), and the locals that take a register at optimisation level 1
/// and debug level 1 (`certain`).
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

/// Globals whose members or calls Luau may fold into constants at optimisation level 2 (builtin libraries and
/// functions).
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

/// A constant Luau folds a value into at optimisation level 1: its type, and its value where it decides something
/// (truthiness, a comparison).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Const {
    Nil,
    Bool(Option<bool>),
    Number(Option<f64>),
    String,
    /// A constant of a type not known here: an `and` / `or` or if-expression whose condition's value is unknown
    /// here, or a builtin Luau may fold at `--!optimize 2`. It counts as folding, the side that never makes a false
    /// error.
    Unknown,
}

impl Const {
    fn truthy(self) -> Option<bool> {
        match self {
            Const::Nil => Some(false),
            Const::Bool(value) => value,
            Const::Number(_) | Const::String => Some(true),
            Const::Unknown => None,
        }
    }

    /// One of two constants (`other` may not fold): their common type, else unknown.
    fn either(self, other: Option<Const>) -> Const {
        match (self, other) {
            (a, Some(b)) if a == b => a,
            (Const::Bool(_), Some(Const::Bool(_))) => Const::Bool(None),
            (Const::Number(_), Some(Const::Number(_))) => Const::Number(None),
            _ => Const::Unknown,
        }
    }
}

/// The value of a number literal's text, where Rust reads it the same way (decimal and exponent forms).
fn number_value(text: &str) -> Option<f64> {
    text.parse().ok()
}

/// `left operator right` on two constants, as Luau's constant folding (`foldBinary`) does it.
fn fold_binary(operator: BinaryOperator, left: Const, right: Const) -> Option<Const> {
    use BinaryOperator as Op;
    let unknown = left == Const::Unknown || right == Const::Unknown;
    let numbers = match (left, right) {
        (Const::Number(a), Const::Number(b)) => Some(a.zip(b)),
        _ => None,
    };
    match operator {
        Op::Add | Op::Sub | Op::Mul | Op::Div | Op::FloorDiv | Op::Mod | Op::Pow => match numbers {
            Some(values) => Some(Const::Number(values.map(|(a, b)| match operator {
                Op::Add => a + b,
                Op::Sub => a - b,
                Op::Mul => a * b,
                Op::Div => a / b,
                Op::FloorDiv => (a / b).floor(),
                Op::Mod => a - (a / b).floor() * b,
                _ => a.powf(b),
            }))),
            None if unknown
                && matches!(left, Const::Number(_) | Const::Unknown)
                && matches!(right, Const::Number(_) | Const::Unknown) =>
            {
                Some(Const::Unknown)
            }
            None => None,
        },
        // strings only: `"a" .. 1` is not folded
        Op::Concat => match (left, right) {
            (Const::String, Const::String) => Some(Const::String),
            (Const::String | Const::Unknown, Const::String | Const::Unknown) => Some(Const::Unknown),
            _ => None,
        },
        // numbers only: `"a" < "b"` is not folded
        Op::Lt | Op::Le | Op::Gt | Op::Ge => match numbers {
            Some(values) => Some(Const::Bool(values.map(|(a, b)| match operator {
                Op::Lt => a < b,
                Op::Le => a <= b,
                Op::Gt => a > b,
                _ => a >= b,
            }))),
            None if matches!(left, Const::Number(_) | Const::Unknown)
                && matches!(right, Const::Number(_) | Const::Unknown) =>
            {
                Some(Const::Bool(None))
            }
            None => None,
        },
        Op::Eq | Op::Ne => {
            let equal = match (left, right) {
                _ if unknown => None,
                (Const::Number(a), Const::Number(b)) => a.zip(b).map(|(a, b)| a == b),
                (Const::Nil, Const::Nil) => Some(true),
                (Const::Bool(a), Const::Bool(b)) => a.zip(b).map(|(a, b)| a == b),
                (Const::String, Const::String) => None,
                _ => Some(false),
            };
            Some(Const::Bool(if operator == Op::Eq { equal } else { equal.map(|e| !e) }))
        }
        // handled with their operands' truthiness
        Op::And | Op::Or => None,
    }
}

/// A local binding, in walk order: both passes walk the file the same way, so its index is the same in both.
struct Binding {
    /// The depth of the function that declares it (0: the main chunk).
    depth: usize,
    /// The constant Luau folds every use of it into at optimisation level 1 and debug level 1 (the second pass).
    constant: Option<Const>,
}

/// A function's counts, for its diagnostics.
struct FunctionReport {
    /// The index of the main chunk's statement that contains it.
    top: usize,
    /// Its name as written (`g`, `a.b`, `A:m`), or `None` for an anonymous function.
    name: Option<String>,
    registers: Registers,
    upvalues: u32,
}

/// The counts of a complete file.
struct Analysis {
    main: Registers,
    /// For each count (conservative, certain), the index of the main chunk's statement where it first passes the
    /// local limit.
    passed: [Option<usize>; 2],
    functions: Vec<FunctionReport>,
}

/// Walks a file in lexical order with its scopes, resolving every use of a local to its binding. The first pass
/// finds the bindings written anywhere, which the second needs from the start (a write can follow the declaration).
struct Walker<'a> {
    /// Luau may fold constants (not at `--!optimize 0`).
    folds: bool,
    /// `--!optimize 2` or higher: builtin calls and reads may fold, and loops with literal bounds may be unrolled.
    optimize_2: bool,
    /// Per binding: written anywhere. The first pass fills it, the second reads it.
    written: Vec<bool>,
    bindings: Vec<Binding>,
    /// The locals in scope, innermost last.
    scopes: Vec<FxHashMap<Name<'a>, usize>>,
    /// The functions being walked, outermost (the main chunk) first: the bindings of enclosing functions each uses.
    upvalues: Vec<FxHashSet<usize>>,
    /// The index of the main chunk's statement being walked.
    top: usize,
    functions: Vec<FunctionReport>,
}

impl<'a> Walker<'a> {
    fn new(level: u32, written: Vec<bool>) -> Walker<'a> {
        Walker {
            folds: level != 0,
            optimize_2: level >= 2,
            written,
            bindings: Vec::new(),
            scopes: Vec::new(),
            upvalues: Vec::new(),
            top: 0,
            functions: Vec::new(),
        }
    }

    fn lookup(&self, name: Name<'a>) -> Option<usize> {
        self.scopes.iter().rev().find_map(|scope| scope.get(&name).copied())
    }

    /// Declares a local in the innermost scope; its constant is set by the caller.
    fn bind(&mut self, name: Name<'a>) -> usize {
        let index = self.bindings.len();
        self.bindings.push(Binding { depth: self.upvalues.len() - 1, constant: None });
        if self.written.len() <= index {
            self.written.push(false);
        }
        self.scopes.last_mut().expect("a scope").insert(name, index);
        index
    }

    /// A use of a local: an upvalue of every function between its declaration and here.
    fn use_binding(&mut self, index: usize) {
        let depth = self.bindings[index].depth;
        for upvalues in &mut self.upvalues[depth + 1..] {
            upvalues.insert(index);
        }
    }

    fn use_name(&mut self, name: Name<'a>) {
        if let Some(index) = self.lookup(name) {
            self.use_binding(index);
        }
    }

    /// A write to a name (an assignment target, `function name()`).
    fn write(&mut self, name: Name<'a>) {
        if let Some(index) = self.lookup(name) {
            self.written[index] = true;
            self.use_binding(index);
        }
    }

    fn is_builtin(&self, e: &'a Expr) -> bool {
        let global_builtin = |e: &'a Expr| match e {
            Expr::Identifier(id) => is_builtin_global(&id.name) && self.lookup(Name::Text(&id.name)).is_none(),
            _ => false,
        };
        match e {
            Expr::PropertyAccess(access) => global_builtin(&access.expression),
            Expr::ComputedIndex(access) => global_builtin(&access.expression),
            e => global_builtin(e),
        }
    }

    /// The constant Luau folds `e` into at optimisation level 1 (or at the file's `--!optimize 2`), if any.
    fn fold(&self, e: &'a Expr) -> Option<Const> {
        match e {
            Expr::Nil => Some(Const::Nil),
            Expr::True => Some(Const::Bool(Some(true))),
            Expr::False => Some(Const::Bool(Some(false))),
            Expr::Number(n) => Some(Const::Number(number_value(&n.value))),
            Expr::String(_) | Expr::LateString(_) => Some(Const::String),
            // `{1}` is not folded into a string: only string parts are
            Expr::InterpolatedString(s) => s
                .parts
                .iter()
                .all(|part| match part {
                    InterpolatedStringPart::Text(_) => true,
                    InterpolatedStringPart::Expression(e) => {
                        matches!(self.fold(e), Some(Const::String | Const::Unknown))
                    }
                })
                .then_some(Const::String),
            Expr::Parenthesized(p) => self.fold(&p.expression),
            Expr::Unary(u) => {
                let value = self.fold(&u.expression)?;
                match (u.operator, value) {
                    (UnaryOperator::Not, value) => Some(Const::Bool(value.truthy().map(|t| !t))),
                    (UnaryOperator::Neg, Const::Number(n)) => Some(Const::Number(n.map(|n| -n))),
                    (UnaryOperator::Len, Const::String) => Some(Const::Number(None)),
                    (_, Const::Unknown) => Some(Const::Unknown),
                    _ => None,
                }
            }
            Expr::Binary(b) => {
                let left = self.fold(&b.left)?;
                match (b.operator, left.truthy()) {
                    (BinaryOperator::And, Some(true)) | (BinaryOperator::Or, Some(false)) => self.fold(&b.right),
                    (BinaryOperator::And, Some(false)) | (BinaryOperator::Or, Some(true)) => Some(left),
                    (BinaryOperator::And | BinaryOperator::Or, None) => Some(left.either(self.fold(&b.right))),
                    (operator, _) => fold_binary(operator, left, self.fold(&b.right)?),
                }
            }
            Expr::If(i) => match self.fold(&i.condition)?.truthy() {
                Some(true) => self.fold(&i.expression),
                Some(false) => self.fold(&i.alternative),
                None => match (self.fold(&i.expression), self.fold(&i.alternative)) {
                    (None, None) => None,
                    (Some(a), b) => Some(a.either(b)),
                    (None, Some(b)) => Some(b.either(None)),
                },
            },
            Expr::Identifier(_) | Expr::TemporaryIdentifier(_) => {
                Name::used(e).and_then(|name| self.lookup(name)).and_then(|index| self.bindings[index].constant)
            }
            Expr::Call(call)
                if self.optimize_2
                    && self.is_builtin(&call.expression)
                    && call.args.iter().all(|a| self.fold(a).is_some()) =>
            {
                Some(Const::Unknown)
            }
            Expr::PropertyAccess(_) | Expr::ComputedIndex(_) if self.optimize_2 && self.is_builtin(e) => {
                Some(Const::Unknown)
            }
            _ => None,
        }
    }

    /// Walks the main chunk.
    fn run(mut self, statements: &'a [Stmt]) -> (Analysis, Vec<bool>) {
        self.upvalues.push(Default::default());
        self.scopes.push(FxHashMap::default());
        let (main, passed) = self.top(statements, Registers::default(), true);
        (Analysis { main, passed, functions: self.functions }, self.written)
    }

    /// A function: its parameters (and `self`) and body in a new frame, then its report.
    fn function(
        &mut self,
        name: Option<String>,
        parameters: &'a [AnyIdentifier],
        implicit_self: bool,
        body: &'a [Stmt],
    ) {
        self.upvalues.push(Default::default());
        self.scopes.push(FxHashMap::default());
        if implicit_self {
            self.bind(Name::Text("self"));
        }
        for id in parameters {
            self.bind(Name::of(id));
        }
        let entry = Registers::names(parameters.len() + usize::from(implicit_self), true);
        let (registers, _) = self.top(body, entry, false);
        self.scopes.pop();
        let upvalues = self.upvalues.pop().expect("the function's frame").len() as u32;
        self.functions.push(FunctionReport { top: self.top, name, registers, upvalues });
    }

    /// The uses and functions in an expression; `name` names a function value (`local g = function`).
    fn expr(&mut self, e: &'a Expr, name: Option<String>) {
        match e {
            Expr::Identifier(_) | Expr::TemporaryIdentifier(_) => {
                if let Some(name) = Name::used(e) {
                    self.use_name(name);
                }
            }
            Expr::Function(f) => self.function(name, &f.parameters, false, &f.statements),
            Expr::ComputedIndex(c) => {
                self.expr(&c.expression, None);
                self.expr(&c.index, None);
            }
            Expr::PropertyAccess(p) => self.expr(&p.expression, None),
            Expr::Call(c) => {
                self.expr(&c.expression, None);
                c.args.iter().for_each(|e| self.expr(e, None));
            }
            Expr::MethodCall(m) => {
                self.expr(&m.expression, None);
                m.args.iter().for_each(|e| self.expr(e, None));
            }
            Expr::Parenthesized(p) => self.expr(&p.expression, name),
            Expr::Binary(b) => {
                self.expr(&b.left, None);
                self.expr(&b.right, None);
            }
            Expr::Unary(u) => self.expr(&u.expression, None),
            Expr::If(i) => {
                self.expr(&i.condition, None);
                self.expr(&i.expression, None);
                self.expr(&i.alternative, None);
            }
            Expr::InterpolatedString(s) => {
                for part in &s.parts {
                    if let InterpolatedStringPart::Expression(e) = part {
                        self.expr(e, None);
                    }
                }
            }
            Expr::Array(items) | Expr::Set(items) => items.iter().for_each(|e| self.expr(e, None)),
            Expr::Map(fields) => fields.iter().for_each(|f| {
                self.expr(&f.index, None);
                self.expr(&f.value, None);
            }),
            Expr::MixedTable(fields) => fields.iter().for_each(|f| match f {
                TableField::Value(e) => self.expr(e, None),
                TableField::Field(f) => {
                    self.expr(&f.index, None);
                    self.expr(&f.value, None);
                }
            }),
            Expr::None
            | Expr::Nil
            | Expr::False
            | Expr::True
            | Expr::Number(_)
            | Expr::String(_)
            | Expr::LateString(_)
            | Expr::VarArgs => {}
        }
    }

    /// An assignment target, or the name of `function name()`: a write to a local, or uses.
    fn target(&mut self, e: &'a Expr) {
        match Name::used(e) {
            Some(name) => self.write(name),
            None => self.expr(e, None),
        }
    }

    /// A `local` statement: its values, then its names. Luau folds the statement into no register when every name
    /// is a constant and there are no more values than names, all or nothing.
    fn declaration(&mut self, declaration: &'a VariableDeclaration) -> Registers {
        for (i, value) in declaration.right.iter().enumerate() {
            self.expr(value, declaration.left.get(i).and_then(describe_id));
        }
        let multiple_results =
            matches!(declaration.right.last(), Some(Expr::Call(_) | Expr::MethodCall(_) | Expr::VarArgs));
        let values: Vec<Option<Const>> = (0..declaration.left.len())
            .map(|i| match declaration.right.get(i) {
                _ if !self.folds => None,
                Some(value) => self.fold(value),
                None if multiple_results => None,
                None => Some(Const::Nil),
            })
            .collect();
        let mut folded = declaration.right.len() <= declaration.left.len();
        for (id, value) in declaration.left.iter().zip(values) {
            let index = self.bind(Name::of(id));
            let constant = value.filter(|_| !self.written[index]);
            folded &= constant.is_some();
            self.bindings[index].constant = constant;
        }
        Registers::names(declaration.left.len(), !folded)
    }

    /// What a statement declares in its block (binding the names), and the most registers live inside its nested
    /// blocks, on top of `active`.
    fn statement(&mut self, statement: &'a Stmt, active: Registers) -> (Registers, Registers) {
        let none = Registers::default();
        match statement {
            Stmt::Assignment(a) => {
                for (i, value) in a.right.iter().enumerate() {
                    self.expr(value, a.left.get(i).and_then(describe));
                }
                a.left.iter().for_each(|target| self.target(target));
                (none, active)
            }
            Stmt::Break | Stmt::Continue | Stmt::Comment(_) => (none, active),
            Stmt::Call(e) => {
                self.expr(e, None);
                (none, active)
            }
            Stmt::Do(block) => (none, self.block(&block.statements, active)),
            Stmt::While(w) => {
                self.expr(&w.condition, None);
                (none, self.block(&w.statements, active))
            }
            Stmt::Repeat(r) => {
                // the condition sees the block's locals
                self.scopes.push(FxHashMap::default());
                let (most, _) = self.top(&r.statements, active, false);
                self.expr(&r.condition, None);
                self.scopes.pop();
                (none, most)
            }
            Stmt::If(i) => (none, self.if_statement(i, active)),
            Stmt::NumericFor(f) => {
                self.expr(&f.start, None);
                self.expr(&f.end, None);
                if let Some(step) = &f.step {
                    self.expr(step, None);
                }
                // only optimisation level 2 unrolls a loop with literal bounds (its variable then takes no register)
                let literal = |e: &Expr| matches!(e, Expr::Number(_));
                let unrollable =
                    self.optimize_2 && literal(&f.start) && literal(&f.end) && f.step.as_ref().is_none_or(literal);
                self.scopes.push(FxHashMap::default());
                self.bind(Name::of(&f.id));
                let most = self.block(&f.statements, active.add(Registers::names(1, !unrollable)));
                self.scopes.pop();
                (none, most)
            }
            Stmt::For(f) => {
                self.expr(&f.expression, None);
                self.scopes.push(FxHashMap::default());
                for id in &f.ids {
                    self.bind(Name::of(id));
                }
                let most = self.block(&f.statements, active.add(Registers::names(f.ids.len(), true)));
                self.scopes.pop();
                (none, most)
            }
            Stmt::FunctionDeclaration(f) => {
                // `local function f` is in scope in its own body; `function f()` writes `f`
                let declared = match (f.localize, Name::used(&f.name)) {
                    (true, Some(name)) => {
                        self.bind(name);
                        Registers::names(1, true)
                    }
                    _ => {
                        self.target(&f.name);
                        none
                    }
                };
                self.function(describe(&f.name), &f.parameters, false, &f.statements);
                (declared, active)
            }
            Stmt::MethodDeclaration(m) => {
                self.expr(&m.expression, None);
                let name = describe(&m.expression).map(|object| format!("{object}:{}", m.name));
                self.function(name, &m.parameters, true, &m.statements);
                (none, active)
            }
            Stmt::VariableDeclaration(declaration) => (self.declaration(declaration), active),
            Stmt::Return(r) => {
                r.expressions.iter().for_each(|e| self.expr(e, None));
                (none, active)
            }
        }
    }

    fn if_statement(&mut self, statement: &'a IfStatement, active: Registers) -> Registers {
        self.expr(&statement.condition, None);
        let then = self.block(&statement.statements, active);
        match &statement.else_body {
            ElseBody::If(next) => then.max(self.if_statement(next, active)),
            ElseBody::Block(statements) => then.max(self.block(statements, active)),
        }
    }

    /// A nested block's pressure, on top of `active`.
    fn block(&mut self, statements: &'a [Stmt], active: Registers) -> Registers {
        self.scopes.push(FxHashMap::default());
        let (most, _) = self.top(statements, active, false);
        self.scopes.pop();
        most
    }

    /// A block's pressure in the current scope, and for each count (conservative, certain) the index of the
    /// statement where it first passes the limit. `main`: the main chunk's own block.
    fn top(&mut self, statements: &'a [Stmt], start: Registers, main: bool) -> (Registers, [Option<usize>; 2]) {
        let mut active = start;
        let mut most = active;
        let mut passed = [None; 2];
        for (i, statement) in statements.iter().enumerate() {
            if main {
                self.top = i;
            }
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

/// A function's name as written: an identifier, a temporary (`_name`), or a property access on one (`a.b`).
fn describe(e: &Expr) -> Option<String> {
    match e {
        Expr::Identifier(id) => Some(id.name.clone()),
        Expr::TemporaryIdentifier(id) => Some(format!("_{}", id.name)),
        Expr::PropertyAccess(p) => describe(&p.expression).map(|object| format!("{object}.{}", p.name)),
        _ => None,
    }
}

/// A local's name as written (`_name` for a temporary).
fn describe_id(id: &AnyIdentifier) -> Option<String> {
    Some(match id {
        AnyIdentifier::Identifier(id) => id.name.clone(),
        AnyIdentifier::Temporary(id) => format!("_{}", id.name),
    })
}

/// The counts of a complete file at its optimisation level.
fn analyse(statements: &[Stmt], level: u32) -> Analysis {
    let (_, written) = Walker::new(level, Vec::new()).run(statements);
    Walker::new(level, written).run(statements).0
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

/// Checks a complete file a plugin emitted anything into (`plugin_file.emitted`): its main chunk (spilling the
/// `hoist_spillable` locals when it may pass Luau's local limit) and each of its functions (locals and upvalues).
/// Files no plugin emitted into are left as roblox-ts leaves them.
pub(crate) fn check_main_chunk(state: &mut TransformState<'_>, statements: &mut Vec<Stmt>) {
    if !state.plugin_file.emitted {
        return;
    }
    let level = optimize_level(statements);
    let mut analysis = analyse(statements, level);
    let mut spilled_at = None;
    if analysis.main.conservative > LUAU_MAX_LOCALS && !state.plugin_file.spillable.is_empty() {
        spilled_at = spill(state, statements);
        analysis = analyse(statements, level);
    }
    let mut diagnostics = Vec::new();
    if let Some(diagnostic) = limit_diagnostic(analysis.main, analysis.passed, "its main chunk", |at| {
        at.and_then(|at| statement_node(state, at, spilled_at)).unwrap_or_else(|| state.source_file.as_node())
    }) {
        diagnostics.push(diagnostic);
    }
    for function in &analysis.functions {
        let anchor = || statement_node(state, function.top, spilled_at).unwrap_or_else(|| state.source_file.as_node());
        let what = match &function.name {
            Some(name) => format!("its function `{name}`"),
            None => "one of its anonymous functions".to_owned(),
        };
        if let Some(diagnostic) = limit_diagnostic(function.registers, [Some(0), Some(0)], &what, |_| anchor()) {
            diagnostics.push(diagnostic);
        }
        if function.upvalues > LUAU_MAX_UPVALUES {
            let count = function.upvalues;
            let subject = match &function.name {
                Some(name) => format!("The function `{name}` in this file"),
                None => "An anonymous function in this file".to_owned(),
            };
            diagnostics.push(PluginDiagnostic::warning(
                anchor(),
                format!(
                    "{subject} uses {count} locals of the functions around it, and Luau allows at most \
                     {LUAU_MAX_UPVALUES} upvalues per function: it fails to load (\"Out of upvalue registers\") \
                     unless Luau folds enough of them as constants (optimisation level 1 or higher at debug level 1)."
                ),
            ));
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
                "This file declares {count} locals live at once in {what} ({} of them take a register even where \
                 Luau folds constant locals), and Luau allows at most {LUAU_MAX_LOCALS} registers: it loads only \
                 where Luau folds them (optimisation level 1 or higher at debug level 1), and fails to load at \
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

#[cfg(test)]
mod tests {
    //! Each case matches a Luau probe loaded with Lune at optimisation levels 0–2 and debug levels 1–2: `certain`
    //! is what fails at level 1, debug level 1.
    use rbxts_luau::{
        AssignmentOperator as A, BinaryOperator as B, InterpolatedStringPart as Part, UnaryOperator as U,
    };

    use super::*;

    fn f() -> Expr {
        create::call(create::id("f"), vec![])
    }

    fn n(value: f64) -> Expr {
        create::number(value)
    }

    /// `local x0 = value` … `local x<count - 1> = value`
    fn locals(count: usize, value: &dyn Fn() -> Expr) -> Vec<Stmt> {
        (0..count).map(|i| create::local(create::any_id(format!("x{i}")), Some(value()))).collect()
    }

    fn main(statements: &[Stmt]) -> Registers {
        analyse(statements, 1).main
    }

    #[test]
    fn values_fold_as_luau_folds_them_at_level_1() {
        let cases: Vec<(&str, &dyn Fn() -> Expr, u32)> = vec![
            ("`abc`", &|| create::interpolated_string(vec![Part::Text("abc".into())]), 0),
            (
                "`a{\"b\"}`",
                &|| create::interpolated_string(vec![Part::Text("a".into()), Part::Expression(create::string("b"))]),
                0,
            ),
            ("`a{1}`", &|| create::interpolated_string(vec![Part::Text("a".into()), Part::Expression(n(1.0))]), 201),
            ("-1", &|| n(-1.0), 0),
            ("1 + 6", &|| create::binary(n(1.0), B::Add, n(6.0)), 0),
            ("0 / 0", &|| n(f64::NAN), 0),
            ("math.huge", &|| n(f64::INFINITY), 201),
            ("\"a\" .. \"b\"", &|| create::binary(create::string("a"), B::Concat, create::string("b")), 0),
            ("\"a\" .. 1", &|| create::binary(create::string("a"), B::Concat, n(1.0)), 201),
            ("1 < 2", &|| create::binary(n(1.0), B::Lt, n(2.0)), 0),
            ("\"a\" < \"b\"", &|| create::binary(create::string("a"), B::Lt, create::string("b")), 201),
            ("1 == \"a\"", &|| create::binary(n(1.0), B::Eq, create::string("a")), 0),
            ("not 1", &|| create::unary(U::Not, n(1.0)), 0),
            ("#\"abc\"", &|| create::unary(U::Len, create::string("abc")), 0),
            ("#1", &|| create::unary(U::Len, n(1.0)), 201),
            ("-\"1\"", &|| create::unary(U::Neg, create::string("1")), 201),
            ("true and f()", &|| create::binary(create::bool(true), B::And, f()), 201),
            ("false and f()", &|| create::binary(create::bool(false), B::And, f()), 0),
            ("1 or f()", &|| create::binary(n(1.0), B::Or, f()), 0),
            ("nil or f()", &|| create::binary(create::nil(), B::Or, f()), 201),
            ("1 < 2 and f()", &|| create::binary(create::binary(n(1.0), B::Lt, n(2.0)), B::And, f()), 201),
            ("if true then 1 else f()", &|| create::if_expr(create::bool(true), n(1.0), f()), 0),
            ("if false then 1 else f()", &|| create::if_expr(create::bool(false), n(1.0), f()), 201),
            ("(1)", &|| create::paren(n(1.0)), 0),
            ("math.abs(1)", &|| create::call(create::property(create::id("math"), "abs"), vec![n(1.0)]), 201),
            ("math.pi", &|| create::property(create::id("math"), "pi"), 201),
            ("G", &|| create::id("G"), 201),
        ];
        for (text, value, certain) in cases {
            assert_eq!(main(&locals(201, value)), Registers { conservative: 201, certain }, "{text}");
        }
        // at --!optimize 0 nothing folds; at 2 builtins may
        assert_eq!(analyse(&locals(201, &|| n(1.0)), 0).main.certain, 201);
        assert_eq!(analyse(&locals(201, &|| create::property(create::id("math"), "pi")), 2).main.certain, 0);
    }

    #[test]
    fn statements_fold_whole_and_names_resolve_by_binding() {
        // more values than names, a missing value (nil), a call's results
        let extra: Vec<Stmt> =
            (0..201).map(|i| create::locals(vec![create::any_id(format!("x{i}"))], vec![n(1.0); 2])).collect();
        assert_eq!(main(&extra).certain, 201);
        let pair = |i: usize| vec![create::any_id(format!("a{i}")), create::any_id(format!("b{i}"))];
        let missing: Vec<Stmt> = (0..101).map(|i| create::locals(pair(i), vec![n(1.0)])).collect();
        assert_eq!(main(&missing).certain, 0);
        let results: Vec<Stmt> = (0..101).map(|i| create::locals(pair(i), vec![f()])).collect();
        assert_eq!(main(&results).certain, 202);

        // a use of a constant local folds; of a local written anywhere, it does not
        let y = || create::id("y");
        let mut alias = vec![create::local(create::any_id("y"), Some(n(1.0)))];
        alias.extend(locals(201, &y));
        assert_eq!(main(&alias).certain, 0);
        let mut written = alias.clone();
        written.insert(1, create::assignment(y(), A::Assign, f()));
        assert_eq!(main(&written).certain, 202);
        let mut later = locals(201, &|| n(1.0));
        later.extend((0..201).map(|i| create::assignment(create::id(format!("x{i}")), A::Add, f())));
        assert_eq!(main(&later).certain, 201);
        // a write in a function counts too
        let mut in_function = locals(201, &|| n(1.0));
        let body = (0..201).map(|i| create::assignment(create::id(format!("x{i}")), A::Assign, f()));
        in_function.push(create::function_declaration(true, create::id("g"), vec![], false, body.collect()));
        assert_eq!(main(&in_function).certain, 202);
        // the inner locals are written, not the outer ones of the same names
        let mut shadowed = locals(201, &|| n(1.0));
        let inner = (0..201).flat_map(|i| {
            [
                create::local(create::any_id(format!("x{i}")), Some(n(2.0))),
                create::assignment(create::id(format!("x{i}")), A::Assign, f()),
            ]
        });
        shadowed.push(create::do_stmt(inner.collect()));
        assert_eq!(main(&shadowed), Registers { conservative: 402, certain: 201 });
    }

    #[test]
    fn loop_variables_and_functions() {
        // a loop variable takes a register at level 1 even with literal bounds (level 2 may unroll the loop)
        let mut fits = locals(199, &f);
        fits.push(create::numeric_for(create::any_id("i"), n(1.0), n(3.0), None, vec![]));
        fits.push(create::local(create::any_id("q"), Some(f())));
        assert_eq!(main(&fits).certain, 200);
        let mut past = locals(200, &f);
        past.push(create::numeric_for(create::any_id("i"), n(1.0), n(3.0), None, vec![]));
        assert_eq!(main(&past).certain, 201);
        assert_eq!(analyse(&past, 2).main.certain, 200);

        // a function sees the constants of the functions around it, not its parameters
        let body = || locals(201, &|| create::id("y"));
        let outer = vec![
            create::local(create::any_id("y"), Some(n(1.0))),
            create::function_declaration(true, create::id("g"), vec![], false, body()),
        ];
        let analysis = analyse(&outer, 1);
        assert_eq!(analysis.functions[0].name.as_deref(), Some("g"));
        assert_eq!(analysis.functions[0].registers, Registers { conservative: 201, certain: 0 });
        let parameter = create::function_declaration(true, create::id("g"), vec![create::any_id("y")], false, body());
        assert_eq!(analyse(&[parameter], 1).functions[0].registers.certain, 202);
    }

    #[test]
    fn upvalues_are_the_enclosing_locals_a_function_and_its_nested_functions_use() {
        let uses =
            |prefix: &str, count: usize| (0..count).map(|i| create::id(format!("{prefix}{i}"))).collect::<Vec<_>>();
        let names = |prefix: &str, count: usize| {
            (0..count).map(|i| create::local(create::any_id(format!("{prefix}{i}")), Some(f()))).collect::<Vec<_>>()
        };
        // the main chunk declares a0…a149; `o` declares b0…b59 and returns a function that uses all of them
        let mut statements = names("a", 150);
        let mut body = names("b", 60);
        let mut used = uses("a", 150);
        used.extend(uses("b", 60));
        let inner =
            create::function(vec![], false, vec![create::call_statement(create::call(create::id("print"), used))]);
        body.push(create::return_stmt(inner));
        statements.push(create::function_declaration(true, create::id("o"), vec![], false, body));
        let analysis = analyse(&statements, 1);
        let upvalues: Vec<(Option<&str>, u32)> =
            analysis.functions.iter().map(|f| (f.name.as_deref(), f.upvalues)).collect();
        assert_eq!(upvalues, vec![(None, 210), (Some("o"), 150)]);
    }
}
