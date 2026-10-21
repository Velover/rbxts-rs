//! Names for temporary identifiers (`solveTempIds`), chosen before rendering.
//!
//! The walk follows TypeScript's `visit` exactly, since the order in which temporary identifiers are met decides
//! who gets `_x` and who gets `_x_1`. Scopes as in TypeScript:
//! - every block (the statements of a function, loop, `do`, `if` / `elseif` / `else` branch) is a scope, but not the
//!   file's top-level list;
//! - functions and `for` loops also get a scope of their own around their parameters (or loop variables) and block.
//!
//! TypeScript finds block edges through `node.parent` (the first and last statement of a block push and pop the
//! block's scope); here the walk knows the block it is in. One quirk is kept: TypeScript pushes a function's own
//! scope before the block scope when a function declaration is the first statement of a block, which gives the
//! same scope tree as pushing the block first.
//!
//! Only declared [`Identifier`]s (locals and parameters) and earlier temporary names count as taken, not loop
//! variables or names used without a declaration. The `in` expression of a generic `for` is not walked (as in
//! TypeScript); a temporary identifier met only there gets a fallback name `_0`, `_1`, … in render order.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use crate::ast::*;

/// The chosen name of each temporary identifier, by id.
#[derive(Default)]
pub(crate) struct TempNames {
    names: FxHashMap<u32, String>,
}

impl TempNames {
    pub(crate) fn get(&self, id: u32) -> Option<&str> {
        self.names.get(&id).map(String::as_str)
    }
}

/// What is being rendered: a file's statements, or one expression (for debugging).
#[derive(Clone, Copy)]
pub(crate) enum Root<'a> {
    Statements(&'a [Stmt]),
    Expr(&'a Expr),
}

pub(crate) fn solve_temp_ids(root: Root) -> TempNames {
    let mut solver = Solver {
        parents: vec![None],
        stack: vec![0],
        declared: FxHashMap::default(),
        temps: Vec::new(),
        unwalked: Vec::new(),
    };
    match root {
        Root::Statements(ast) => solver.statements(ast, false),
        Root::Expr(e) => solver.expr(e),
    }

    let mut names = TempNames::default();
    let mut last_try: FxHashMap<(usize, String), u32> = FxHashMap::default();
    for (temp, scope) in std::mem::take(&mut solver.temps) {
        if names.names.contains_key(&temp.id) {
            continue;
        }
        let separator = if temp.name.is_empty() { "" } else { "_" };
        let mut input = format!("_{}", temp.name);
        // TypeScript records the last try under the name it settled on, so this only helps a name that was free
        let mut i = last_try.get(&(scope, input.clone())).copied().unwrap_or(1);
        while solver.has_id(scope, &input) {
            input = format!("_{}{separator}{i}", temp.name);
            i += 1;
        }
        last_try.insert((scope, input.clone()), i);
        solver.declare(scope, Cow::Owned(input.clone()));
        names.names.insert(temp.id, input);
    }

    if solver.unwalked.iter().any(|e| {
        let mut unnamed = false;
        walk_expr(e, &mut |t| unnamed |= !names.names.contains_key(&t.id));
        unnamed
    }) {
        // `RenderState.getTempName`'s fallback, in the order the renderer meets them
        let mut next = 0;
        let mut fallback = |t: &TemporaryIdentifier| {
            names.names.entry(t.id).or_insert_with(|| {
                next += 1;
                format!("_{}", next - 1)
            });
        };
        match root {
            Root::Statements(ast) => walk_statements(ast, &mut fallback),
            Root::Expr(e) => walk_expr(e, &mut fallback),
        }
    }
    names
}

/// The scopes that declare a name. Most names are declared once, so the first needs no allocation.
struct Declarers {
    first: usize,
    more: Vec<usize>,
}

impl Declarers {
    fn contains(&self, scope: usize) -> bool {
        self.first == scope || self.more.contains(&scope)
    }
}

struct Solver<'a> {
    /// The parent of every scope made, by scope index; `stack` is the current chain.
    parents: Vec<Option<usize>>,
    stack: Vec<usize>,
    /// Declared names (locals, parameters, temporary names given so far) and the scopes they are declared in: one
    /// map for the file rather than a set per scope, since most scopes are blocks that are never asked about.
    declared: FxHashMap<Cow<'a, str>, Declarers>,
    /// Temporary identifiers in visit order, with the scope each was met in.
    temps: Vec<(&'a TemporaryIdentifier, usize)>,
    /// `for … in` expressions, which the visit skips.
    unwalked: Vec<&'a Expr>,
}

impl<'a> Solver<'a> {
    fn current(&self) -> usize {
        *self.stack.last().expect("the file scope is never popped")
    }

    fn push(&mut self) {
        self.parents.push(Some(self.current()));
        self.stack.push(self.parents.len() - 1);
    }

    fn pop(&mut self) {
        self.stack.pop();
    }

    fn declare(&mut self, scope: usize, name: Cow<'a, str>) {
        match self.declared.get_mut(name.as_ref()) {
            Some(d) => {
                if !d.contains(scope) {
                    d.more.push(scope);
                }
            }
            None => {
                self.declared.insert(name, Declarers { first: scope, more: Vec::new() });
            }
        }
    }

    /// `scopeHasId`: declared in `scope` or one of its ancestors.
    fn has_id(&self, mut scope: usize, id: &str) -> bool {
        let Some(declarers) = self.declared.get(id) else { return false };
        loop {
            if declarers.contains(scope) {
                return true;
            }
            match self.parents[scope] {
                Some(parent) => scope = parent,
                None => return false,
            }
        }
    }

    fn register(&mut self, ids: &'a [AnyIdentifier]) {
        let scope = self.current();
        for id in ids {
            if let AnyIdentifier::Identifier(id) = id {
                self.declare(scope, Cow::Borrowed(&id.name));
            }
        }
    }

    fn statements(&mut self, statements: &'a [Stmt], is_block: bool) {
        let scoped = is_block && !statements.is_empty();
        if scoped {
            self.push();
        }
        for s in statements {
            self.statement(s);
        }
        if scoped {
            self.pop();
        }
    }

    fn any_identifier(&mut self, id: &'a AnyIdentifier) {
        if let AnyIdentifier::Temporary(t) = id {
            self.temps.push((t, self.current()));
        }
    }

    /// A function's own scope: its parameters, then its body.
    fn function(&mut self, name: Option<&'a Expr>, parameters: &'a [AnyIdentifier], statements: &'a [Stmt]) {
        self.push();
        self.register(parameters);
        if let Some(name) = name {
            self.expr(name);
        }
        for p in parameters {
            self.any_identifier(p);
        }
        self.statements(statements, true);
        self.pop();
    }

    fn if_statement(&mut self, node: &'a IfStatement) {
        self.expr(&node.condition);
        self.statements(&node.statements, true);
        match &node.else_body {
            ElseBody::If(elseif) => self.if_statement(elseif),
            ElseBody::Block(statements) => self.statements(statements, true),
        }
    }

    fn statement(&mut self, s: &'a Stmt) {
        match s {
            Stmt::Assignment(node) => {
                self.exprs(&node.left);
                self.exprs(&node.right);
            }
            Stmt::Break | Stmt::Continue | Stmt::Comment(_) => {}
            Stmt::Call(e) => self.expr(e),
            Stmt::Do(node) => self.statements(&node.statements, true),
            Stmt::While(node) => {
                self.expr(&node.condition);
                self.statements(&node.statements, true);
            }
            Stmt::Repeat(node) => {
                self.statements(&node.statements, true);
                self.expr(&node.condition);
            }
            Stmt::If(node) => self.if_statement(node),
            Stmt::NumericFor(node) => {
                self.push();
                self.any_identifier(&node.id);
                self.expr(&node.start);
                self.expr(&node.end);
                if let Some(step) = &node.step {
                    self.expr(step);
                }
                self.statements(&node.statements, true);
                self.pop();
            }
            Stmt::For(node) => {
                self.push();
                for id in &node.ids {
                    self.any_identifier(id);
                }
                self.unwalked.push(&node.expression);
                self.statements(&node.statements, true);
                self.pop();
            }
            Stmt::FunctionDeclaration(node) => self.function(Some(&node.name), &node.parameters, &node.statements),
            Stmt::MethodDeclaration(node) => self.function(Some(&node.expression), &node.parameters, &node.statements),
            Stmt::VariableDeclaration(node) => {
                self.register(&node.left);
                for id in &node.left {
                    self.any_identifier(id);
                }
                self.exprs(&node.right);
            }
            Stmt::Return(node) => self.exprs(&node.expressions),
        }
    }

    fn exprs(&mut self, exprs: &'a [Expr]) {
        for e in exprs {
            self.expr(e);
        }
    }

    fn expr(&mut self, e: &'a Expr) {
        match e {
            Expr::TemporaryIdentifier(t) => self.temps.push((t, self.current())),
            Expr::ComputedIndex(node) => {
                self.expr(&node.expression);
                self.expr(&node.index);
            }
            Expr::PropertyAccess(node) => self.expr(&node.expression),
            Expr::Call(node) => {
                self.expr(&node.expression);
                self.exprs(&node.args);
            }
            Expr::MethodCall(node) => {
                self.expr(&node.expression);
                self.exprs(&node.args);
            }
            Expr::Parenthesized(inner) => self.expr(inner),
            Expr::Function(node) => self.function(None, &node.parameters, &node.statements),
            Expr::Binary(node) => {
                self.expr(&node.left);
                self.expr(&node.right);
            }
            Expr::Unary(node) => self.expr(&node.expression),
            Expr::If(node) => {
                self.expr(&node.condition);
                self.expr(&node.expression);
                self.expr(&node.alternative);
            }
            Expr::InterpolatedString(node) => {
                for part in &node.parts {
                    if let InterpolatedStringPart::Expression(e) = part {
                        self.expr(e);
                    }
                }
            }
            Expr::Array(members) | Expr::Set(members) => self.exprs(members),
            Expr::Map(fields) => {
                for f in fields {
                    self.expr(&f.index);
                    self.expr(&f.value);
                }
            }
            Expr::MixedTable(fields) => {
                for f in fields {
                    match f {
                        TableField::Value(e) => self.expr(e),
                        TableField::Field(f) => {
                            self.expr(&f.index);
                            self.expr(&f.value);
                        }
                    }
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
        }
    }
}

// Every temporary identifier the renderer writes, in the order it writes them. TypeScript renders a map field's
// value before its key, and a numeric for's step only when it is not a literal 1 (which holds no identifiers).

fn walk_statements(statements: &[Stmt], f: &mut impl FnMut(&TemporaryIdentifier)) {
    for s in statements {
        walk_statement(s, f);
    }
}

fn walk_any_identifiers(ids: &[AnyIdentifier], f: &mut impl FnMut(&TemporaryIdentifier)) {
    for id in ids {
        if let AnyIdentifier::Temporary(t) = id {
            f(t);
        }
    }
}

fn walk_statement(s: &Stmt, f: &mut impl FnMut(&TemporaryIdentifier)) {
    match s {
        Stmt::Assignment(node) => {
            walk_exprs(&node.left, f);
            walk_exprs(&node.right, f);
        }
        Stmt::Break | Stmt::Continue | Stmt::Comment(_) => {}
        Stmt::Call(e) => walk_expr(e, f),
        Stmt::Do(node) => walk_statements(&node.statements, f),
        Stmt::While(node) => {
            walk_expr(&node.condition, f);
            walk_statements(&node.statements, f);
        }
        Stmt::Repeat(node) => {
            walk_statements(&node.statements, f);
            walk_expr(&node.condition, f);
        }
        Stmt::If(node) => {
            let mut node = &**node;
            loop {
                walk_expr(&node.condition, f);
                walk_statements(&node.statements, f);
                match &node.else_body {
                    ElseBody::If(elseif) => node = elseif,
                    ElseBody::Block(statements) => break walk_statements(statements, f),
                }
            }
        }
        Stmt::NumericFor(node) => {
            walk_any_identifiers(std::slice::from_ref(&node.id), f);
            walk_expr(&node.start, f);
            walk_expr(&node.end, f);
            if let Some(step) = &node.step {
                walk_expr(step, f);
            }
            walk_statements(&node.statements, f);
        }
        Stmt::For(node) => {
            walk_any_identifiers(&node.ids, f);
            walk_expr(&node.expression, f);
            walk_statements(&node.statements, f);
        }
        Stmt::FunctionDeclaration(node) => {
            walk_expr(&node.name, f);
            walk_any_identifiers(&node.parameters, f);
            walk_statements(&node.statements, f);
        }
        Stmt::MethodDeclaration(node) => {
            walk_expr(&node.expression, f);
            walk_any_identifiers(&node.parameters, f);
            walk_statements(&node.statements, f);
        }
        Stmt::VariableDeclaration(node) => {
            walk_any_identifiers(&node.left, f);
            walk_exprs(&node.right, f);
        }
        Stmt::Return(node) => walk_exprs(&node.expressions, f),
    }
}

fn walk_exprs(exprs: &[Expr], f: &mut impl FnMut(&TemporaryIdentifier)) {
    for e in exprs {
        walk_expr(e, f);
    }
}

fn walk_expr(e: &Expr, f: &mut impl FnMut(&TemporaryIdentifier)) {
    match e {
        Expr::TemporaryIdentifier(t) => f(t),
        Expr::ComputedIndex(node) => {
            walk_expr(&node.expression, f);
            walk_expr(&node.index, f);
        }
        Expr::PropertyAccess(node) => walk_expr(&node.expression, f),
        Expr::Call(node) => {
            walk_expr(&node.expression, f);
            walk_exprs(&node.args, f);
        }
        Expr::MethodCall(node) => {
            walk_expr(&node.expression, f);
            walk_exprs(&node.args, f);
        }
        Expr::Parenthesized(inner) => walk_expr(inner, f),
        Expr::Function(node) => {
            walk_any_identifiers(&node.parameters, f);
            walk_statements(&node.statements, f);
        }
        Expr::Binary(node) => {
            walk_expr(&node.left, f);
            walk_expr(&node.right, f);
        }
        Expr::Unary(node) => walk_expr(&node.expression, f),
        Expr::If(node) => {
            walk_expr(&node.condition, f);
            walk_expr(&node.expression, f);
            walk_expr(&node.alternative, f);
        }
        Expr::InterpolatedString(node) => {
            for part in &node.parts {
                if let InterpolatedStringPart::Expression(e) = part {
                    walk_expr(e, f);
                }
            }
        }
        Expr::Array(members) | Expr::Set(members) => walk_exprs(members, f),
        Expr::Map(fields) => {
            for field in fields {
                walk_expr(&field.value, f);
                walk_expr(&field.index, f);
            }
        }
        Expr::MixedTable(fields) => {
            for field in fields {
                match field {
                    TableField::Value(e) => walk_expr(e, f),
                    TableField::Field(field) => {
                        walk_expr(&field.value, f);
                        walk_expr(&field.index, f);
                    }
                }
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
    }
}

// FxHash (rustc's hasher): the sets and maps here hold short names, for which SipHash is the slow part.
type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

#[derive(Default)]
struct FxHasher {
    hash: u64,
}

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.hash = (self.hash.rotate_left(5) ^ u64::from_le_bytes(word)).wrapping_mul(SEED);
        }
    }

    fn write_u32(&mut self, n: u32) {
        self.write(&n.to_le_bytes());
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}
