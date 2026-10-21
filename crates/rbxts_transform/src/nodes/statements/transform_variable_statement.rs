//! `nodes/statements/transformVariableStatement.ts`

use rbxts_luau::{AssignmentOperator, Expr, Stmt, create};
use tsrs_ast::{Kind, Node, NodeFlags};
use tsrs_core::P;

use crate::classes::prereqs::{Prereqs, as_any_identifier};
use crate::classes::transform_state::TransformState;
use crate::nodes::expressions::transform_expression::transform_expression;
use crate::nodes::expressions::transform_identifier::transform_identifier_defined;
use crate::shared::diagnostics::errors;
use crate::util::check_variable_hoist::check_variable_hoist;
use crate::util::evaluation::facts::{copy_value_facts, get_call_effects, is_constant_reference};
use crate::util::symbols::{is_symbol_mutable, validate_identifier};
use crate::util::wrap_expression_statement::wrap_expression_statement;

/// `transformVariable`: declares (or, for `export let`, assigns) one variable. Returns the expression it is now
/// reachable as.
pub fn transform_variable(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    identifier: P<Node>,
    right: Option<Expr>,
) -> Expr {
    validate_identifier(state, identifier);

    let symbol = state.checker.get_symbol_at_location_exported(identifier).expect("a variable has a symbol");

    // export let
    if is_symbol_mutable(state, symbol)
        && let Some(export_access) = state.get_module_id_property_access(symbol)
    {
        if let Some(right) = right {
            prereqs.push(create::assignment(export_access.clone(), AssignmentOperator::Assign, right));
        }
        return export_access;
    }

    let mut left = transform_identifier_defined(state, identifier);
    if let Some(right) = &right
        && is_constant_reference(&left)
    {
        if let Some(effects) = get_call_effects(right) {
            state.multi_transform_state.function_effects.insert(symbol, effects);
        }
        copy_value_facts(right, &mut left);
    }

    check_variable_hoist(state, identifier, symbol);
    if state.is_hoisted.get(&symbol) == Some(&true) {
        // no need to do `x = nil` if the variable is already created
        if let Some(right) = right {
            prereqs.push(create::assignment(left.clone(), AssignmentOperator::Assign, right));
        }
    } else {
        prereqs.push(create::local(as_any_identifier(left.clone()), right));
    }

    left
}

/// `transformVariableDeclaration`
pub fn transform_variable_declaration(state: &mut TransformState<'_>, node: P<Node>) -> Vec<Stmt> {
    let mut statements = Vec::new();
    let declaration = node.as_variable_declaration();
    let initializer = declaration.initializer();
    let mut value = None;
    if let Some(initializer) = initializer {
        // the value is transformed before checking isHoisted, so references inside it can be hoisted
        let mut initializer_prereqs = Prereqs::new();
        value = Some(transform_expression(state, &mut initializer_prereqs, initializer));
        statements.extend(initializer_prereqs.statements);
    }

    let name = declaration.name;
    if name.kind() == Kind::Identifier {
        let mut variable_prereqs = Prereqs::new();
        transform_variable(state, &mut variable_prereqs, name, value);
        statements.extend(variable_prereqs.statements);
    } else {
        let value = value.expect("a destructuring declaration has an initializer");
        // optimize empty destructure
        if name.elements().is_empty() {
            if !matches!(&value, Expr::Array(members) if members.is_empty()) {
                statements.extend(wrap_expression_statement(value));
            }
            return statements;
        }
        state.unsupported_subtree(name, &format!("{:?} variable", name.kind()));
    }

    statements
}

/// `isVarDeclaration`
pub fn is_var_declaration(node: P<Node>) -> bool {
    !node.flags().intersects(NodeFlags::Const) && !node.flags().intersects(NodeFlags::Let)
}

/// `transformVariableDeclarationList`
pub fn transform_variable_declaration_list(state: &mut TransformState<'_>, node: P<Node>) -> Vec<Stmt> {
    if is_var_declaration(node) {
        state.add_diagnostic(errors::no_var(node));
    }

    let mut statements = Vec::new();
    for &declaration in node.as_variable_declaration_list().declarations.nodes() {
        statements.extend(transform_variable_declaration(state, declaration));
    }
    statements
}

/// `transformVariableStatement`
pub fn transform_variable_statement(state: &mut TransformState<'_>, node: P<Node>) -> Vec<Stmt> {
    transform_variable_declaration_list(state, node.as_variable_statement().declaration_list)
}
