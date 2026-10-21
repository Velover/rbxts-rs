//! Luau's limit of 200 locals in a function, checked on the file's main chunk once the file is lowered (DESIGN
//! §9: the compiler owns the check for all code, its own output included). Past the limit, the locals plugins
//! hoisted with `hoist_spillable` move into one table local first; a file still past it is an error, as it would
//! build and then fail to load ("Out of local registers").

use rustc_hash::FxHashMap;

use rbxts_luau::{AnyIdentifier, ElseBody, Expr, IfStatement, InterpolatedStringPart, Stmt, TableField, create};
use rbxts_plugin::PluginDiagnostic;
use tsrs_ast::Node;
use tsrs_core::P;

use super::backend::to_diagnostic;
use super::file_state::declared_locals;
use crate::classes::prereqs::temp_id;
use crate::classes::transform_state::TransformState;

/// The most locals Luau allows in one function, the main chunk included.
pub const LUAU_MAX_LOCALS: u32 = 200;

/// Checks the main chunk of a file with plugins, before plugin imports and header statements are placed (they are
/// counted as if at the top): spills `hoist_spillable` locals into one table when the chunk is past the limit,
/// then reports an error if it still is.
pub(crate) fn check_main_chunk(state: &mut TransformState<'_>, statements: &mut Vec<Stmt>) {
    let placed_first: u32 = state.plugin_file.imports.iter().filter(|i| i.statement.is_some()).count() as u32
        + state.plugin_file.header.iter().map(declared_locals).sum::<u32>();
    let count = |statements: &[Stmt]| placed_first + statements.iter().map(declared_locals).sum::<u32>();
    if count(statements) <= LUAU_MAX_LOCALS {
        return;
    }
    if !state.plugin_file.spillable.is_empty() {
        spill(state, statements);
    }
    let total = count(statements);
    if total <= LUAU_MAX_LOCALS {
        return;
    }
    // the first top-level statement at which the count passes the limit
    let mut running = placed_first;
    let mut passed_at = statements.len();
    for (i, statement) in statements.iter().enumerate() {
        running += declared_locals(statement);
        if running > LUAU_MAX_LOCALS {
            passed_at = i;
            break;
        }
    }
    let anchor = state
        .plugin_file
        .root_ranges
        .iter()
        .find(|range| range.end > passed_at)
        .map_or_else(|| state.source_file.as_node(), |range| range.node);
    let message = format!(
        "This file declares {total} locals in its main chunk, and Luau allows at most {LUAU_MAX_LOCALS} there: it \
         would build, then fail to load (\"Out of local registers\"). The count passes the limit here."
    );
    state.add_diagnostic(to_diagnostic(&PluginDiagnostic::error(anchor, message)));
}

/// The spilled hoists' table and fields.
struct Spill {
    table: Expr,
    fields: FxHashMap<u32, String>,
}

/// Moves every `hoist_spillable` local whose declaration is a statement of the main chunk into one table local:
/// `local _spill = {}` where the first was declared, `_spill.<hint> = value` for each, and every use of one
/// becomes `_spill.<hint>`.
fn spill(state: &mut TransformState<'_>, statements: &mut Vec<Stmt>) {
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
    let Some(&(first, _)) = declarations.first() else { return };
    let table_id = temp_id(state.plugin_file.seam_hint(state.source_file, "spill"));
    let table = Expr::TemporaryIdentifier(table_id.clone());
    let mut fields = FxHashMap::default();
    let mut taken = rustc_hash::FxHashSet::default();
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
    let mut header = std::mem::take(&mut state.plugin_file.header);
    rewrite_statements(&mut header, &spill, &mut misplaced);
    state.plugin_file.header = header;
    // the table first: `local _spill = {}` before the first field assignment (a root insertion, outside every
    // statement's range)
    statements.insert(first, create::local(table_id, Some(create::map([]))));
    for range in &mut state.plugin_file.root_ranges {
        if range.start >= first {
            range.start += 1;
        }
        if range.end > first {
            range.end += 1;
        }
    }
    if misplaced {
        let node: P<Node> = state.source_file.as_node();
        state.internal_error(
            node,
            "a hoist_spillable local was declared again by plugin code (as a parameter, loop variable or local), so \
             it cannot be spilled into a table",
        );
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
