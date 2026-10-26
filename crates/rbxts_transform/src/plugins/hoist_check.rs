//! What a hoisted value may refer to (`hoist`, `hoist_spillable`, `hoist_spillable_named`). A hoist is declared before
//! the top-level statement around the hook's node, so its value must not use anything that statement declares: a
//! temporary its lowering made so far (a prereq's, roblox-ts's or the plugin's own `push_to_var`), or a local of the
//! code around the node (a parameter, a local of a function or block, `self`, the class's own name). Used there, it
//! would read a global instead (`nil`), silently. The compiler refuses such a value: an internal error naming the
//! plugin.

use rustc_hash::FxHashSet;

use rbxts_luau::{AnyIdentifier, ElseBody, Expr, InterpolatedStringPart, Stmt, TableField};
use tsrs_ast::{Kind, Node, SymbolFlags};
use tsrs_core::P;

use crate::classes::transform_state::TransformState;

/// The temporaries and names an expression uses that it does not declare itself (in a function or a block of its
/// own).
#[derive(Default)]
struct Free<'t> {
    used_temporaries: Vec<u32>,
    declared_temporaries: FxHashSet<u32>,
    used_names: Vec<&'t str>,
    declared_names: FxHashSet<&'t str>,
}

impl<'t> Free<'t> {
    fn declare(&mut self, ids: &'t [AnyIdentifier]) {
        for id in ids {
            match id {
                AnyIdentifier::Temporary(t) => {
                    self.declared_temporaries.insert(t.id);
                }
                AnyIdentifier::Identifier(id) => {
                    self.declared_names.insert(&id.name);
                }
            }
        }
    }

    fn statements(&mut self, statements: &'t [Stmt]) {
        for s in statements {
            self.statement(s);
        }
    }

    fn statement(&mut self, s: &'t Stmt) {
        match s {
            Stmt::Assignment(node) => {
                self.exprs(&node.left);
                self.exprs(&node.right);
            }
            Stmt::Break | Stmt::Continue | Stmt::Comment(_) => {}
            Stmt::Call(e) => self.expr(e),
            Stmt::Do(node) => self.statements(&node.statements),
            Stmt::While(node) => {
                self.expr(&node.condition);
                self.statements(&node.statements);
            }
            Stmt::Repeat(node) => {
                self.statements(&node.statements);
                self.expr(&node.condition);
            }
            Stmt::If(node) => {
                let mut node = &**node;
                loop {
                    self.expr(&node.condition);
                    self.statements(&node.statements);
                    match &node.else_body {
                        ElseBody::If(elseif) => node = elseif,
                        ElseBody::Block(statements) => break self.statements(statements),
                    }
                }
            }
            Stmt::NumericFor(node) => {
                self.declare(std::slice::from_ref(&node.id));
                self.expr(&node.start);
                self.expr(&node.end);
                if let Some(step) = &node.step {
                    self.expr(step);
                }
                self.statements(&node.statements);
            }
            Stmt::For(node) => {
                self.declare(&node.ids);
                self.expr(&node.expression);
                self.statements(&node.statements);
            }
            Stmt::FunctionDeclaration(node) => {
                if node.localize {
                    match &node.name {
                        Expr::TemporaryIdentifier(t) => {
                            self.declared_temporaries.insert(t.id);
                        }
                        Expr::Identifier(id) => {
                            self.declared_names.insert(&id.name);
                        }
                        other => self.expr(other),
                    }
                } else {
                    self.expr(&node.name);
                }
                self.declare(&node.parameters);
                self.statements(&node.statements);
            }
            Stmt::MethodDeclaration(node) => {
                self.expr(&node.expression);
                self.declare(&node.parameters);
                self.statements(&node.statements);
            }
            Stmt::VariableDeclaration(node) => {
                self.declare(&node.left);
                self.exprs(&node.right);
            }
            Stmt::Return(node) => self.exprs(&node.expressions),
        }
    }

    fn exprs(&mut self, exprs: &'t [Expr]) {
        for e in exprs {
            self.expr(e);
        }
    }

    fn expr(&mut self, e: &'t Expr) {
        match e {
            Expr::TemporaryIdentifier(t) => self.used_temporaries.push(t.id),
            Expr::Identifier(id) => self.used_names.push(&id.name),
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
            Expr::Parenthesized(node) => self.expr(&node.expression),
            Expr::Function(node) => {
                self.declare(&node.parameters);
                self.statements(&node.statements);
            }
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
                    match part {
                        InterpolatedStringPart::Expression(e) => self.expr(e),
                        InterpolatedStringPart::Text(_) => {}
                    }
                }
            }
            Expr::Array(members) | Expr::Set(members) => self.exprs(members),
            Expr::Map(fields) => {
                for field in fields {
                    self.expr(&field.index);
                    self.expr(&field.value);
                }
            }
            Expr::MixedTable(fields) => {
                for field in fields {
                    match field {
                        TableField::Value(e) => self.expr(e),
                        TableField::Field(field) => {
                            self.expr(&field.index);
                            self.expr(&field.value);
                        }
                    }
                }
            }
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
}

/// The top-level statement around `node` (its ancestor whose parent is the source file), or the node itself.
fn root_statement(node: P<Node>) -> P<Node> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.kind() == Kind::SourceFile {
            return current;
        }
        current = parent;
    }
    current
}

/// `node` is inside a function, a class, a block or another scope of its own below the file's top level.
fn in_inner_scope(node: P<Node>) -> bool {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == Kind::SourceFile {
            return false;
        }
        if tsrs_ast::is_function_like(n)
            || tsrs_ast::is_class_like(n)
            || matches!(
                n.kind(),
                Kind::Block
                    | Kind::ModuleBlock
                    | Kind::CaseBlock
                    | Kind::CatchClause
                    | Kind::ForStatement
                    | Kind::ForInStatement
                    | Kind::ForOfStatement
                    | Kind::ClassStaticBlockDeclaration
            )
        {
            return true;
        }
        current = n.parent();
    }
    false
}

/// Why `value`, hoisted for the hook at `anchor`, would not read what it reads at the hook's node: it uses a
/// temporary declared within the current top-level statement (made at or after `temp_start`, except the seam's own
/// top-level ones in `top_level`), or a name that, at the hook's node, is a local of that statement (declared in it,
/// or a parameter or local of a function, class or block around the node), or `self` / `super` inside a function or
/// class. `None` when it may be hoisted.
pub(crate) fn hoist_problem(
    state: &mut TransformState<'_>,
    anchor: P<Node>,
    value: &Expr,
    temp_start: u32,
    top_level: &FxHashSet<u32>,
) -> Option<String> {
    let mut free = Free::default();
    free.expr(value);
    for &id in &free.used_temporaries {
        if id >= temp_start && !top_level.contains(&id) && !free.declared_temporaries.contains(&id) {
            return Some(
                "uses a temporary declared in the current statement (a prerequisite's, or one roblox-ts made while \
                 lowering it)"
                    .to_owned(),
            );
        }
    }
    let root = root_statement(anchor);
    let file = tsrs_ast::get_source_file_of_node(anchor);
    for &name in &free.used_names {
        if free.declared_names.contains(name) {
            continue;
        }
        if matches!(name, "self" | "super") {
            if in_inner_scope(anchor) {
                return Some(format!("uses `{name}`, which is a local of the function or class around its node"));
            }
            continue;
        }
        let Some(symbol) = state.checker.resolve_name_exported(name, Some(anchor), SymbolFlags::Value, false) else {
            continue;
        };
        let local = symbol.declarations().iter().any(|&declaration| {
            tsrs_ast::get_source_file_of_node(declaration) == file
                && (root_statement(declaration) == root || in_inner_scope(declaration))
        });
        if local {
            return Some(format!(
                "uses `{name}`, which is declared in the current statement (a parameter or a local of the code around \
                 its node)"
            ));
        }
    }
    None
}
