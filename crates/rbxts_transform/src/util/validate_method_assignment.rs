//! `util/validateMethodAssignment.ts` (the expression and object literal parts; class element checks come with the
//! classes area).

use tsrs_ast::{Kind, Node, NodeFlags};
use tsrs_checker::{ContextFlags, SignatureKind, Type};
use tsrs_core::P;

use crate::classes::transform_state::TransformState;
use crate::shared::diagnostics::errors;
use crate::util::is_method::{is_method, is_method_from_type};
use crate::util::is_valid_method_index_without_call::is_valid_method_index_without_call;
use crate::util::traversal::{skip_downwards, skip_upwards};
use crate::util::types::walk_types;

fn has_call_signatures(state: &mut TransformState<'_>, t: P<Type>) -> bool {
    let mut has = false;
    walk_types(state, t, &mut |state, t| {
        has = has || !state.checker.get_signatures_of_type_exported(t, SignatureKind::Call).is_empty();
    });
    has
}

fn is_zero_parameter_function(state: &mut TransformState<'_>, mut node: P<Node>) -> bool {
    if node.kind() == Kind::PropertyAssignment {
        node = skip_downwards(node.initializer().expect("a property assignment has an initializer"));
    }

    if matches!(node.kind(), Kind::Identifier | Kind::ShorthandPropertyAssignment) {
        let symbol = if node.kind() == Kind::ShorthandPropertyAssignment {
            state.checker.get_shorthand_assignment_value_symbol(Some(node))
        } else {
            state.checker.get_symbol_at_location_exported(node)
        }
        .expect("a function reference has a symbol");
        match symbol.value_declaration() {
            Some(declaration) if declaration.kind() == Kind::VariableDeclaration => {
                let is_const = declaration.parent().is_some_and(|p| p.flags().intersects(NodeFlags::Const));
                let Some(initializer) = declaration.initializer() else { return false };
                if !is_const {
                    return false;
                }
                node = skip_downwards(initializer);
            }
            _ => {
                // overload signatures can hide optional or rest parameters in the implementation
                let implementation = symbol
                    .declarations()
                    .iter()
                    .copied()
                    .find(|d| d.kind() == Kind::FunctionDeclaration && d.body().is_some());
                let Some(implementation) = implementation else { return false };
                node = implementation;
            }
        }
    }

    matches!(node.kind(), Kind::ArrowFunction | Kind::FunctionExpression | Kind::FunctionDeclaration)
        && node.parameters().iter().all(|p| p.name().is_some_and(tsrs_ast::is_this_identifier))
        && !is_method(state, node)
}

fn validate_types(state: &mut TransformState<'_>, node: P<Node>, base_type: P<Type>, assignment_type: P<Type>) {
    if has_call_signatures(state, base_type) && has_call_signatures(state, assignment_type) {
        let assignment_is_method = is_method_from_type(state, node, assignment_type);
        if is_method_from_type(state, node, base_type) != assignment_is_method {
            if assignment_is_method {
                // zero-parameter functions without a receiver ignore the extra receiver argument
                if is_zero_parameter_function(state, node) {
                    return;
                }
                state.add_diagnostic(errors::expected_method_got_function(node));
            } else {
                state.add_diagnostic(errors::expected_function_got_method(node));
            }
        }
    }
}

/// `validateMethodExpression`: a function value used where the other calling convention is expected.
pub fn validate_method_expression(state: &mut TransformState<'_>, node: P<Node>, t: P<Type>) {
    let Some(parent) = skip_upwards(node).parent() else { return };
    // object literal members are checked together with their contextual property types
    if matches!(parent.kind(), Kind::PropertyAssignment | Kind::ShorthandPropertyAssignment)
        || !has_call_signatures(state, t)
    {
        return;
    }

    let contextual_type = state.checker.get_contextual_type_exported(node, ContextFlags::None);
    if let Some(contextual_type) = contextual_type
        && contextual_type != t
    {
        let expression = skip_downwards(node);
        // method extraction already reports why this value cannot be used as a callback
        if matches!(expression.kind(), Kind::PropertyAccessExpression | Kind::ElementAccessExpression)
            && !is_valid_method_index_without_call(state, skip_upwards(node))
            && is_method_from_type(state, node, t)
        {
            return;
        }
        validate_types(state, expression, t, contextual_type);
    }
}

/// `ts.isObjectLiteralElementLike`
fn is_object_literal_element_like(node: P<Node>) -> bool {
    matches!(
        node.kind(),
        Kind::PropertyAssignment
            | Kind::ShorthandPropertyAssignment
            | Kind::SpreadAssignment
            | Kind::MethodDeclaration
            | Kind::GetAccessor
            | Kind::SetAccessor
    )
}

/// `validateObjectLiteralElement`: a member against its contextual property type.
fn validate_object_literal_element(state: &mut TransformState<'_>, node: P<Node>) {
    let t = state.get_type(node);
    let contextual_type =
        state.checker.get_contextual_type_for_object_literal_element_exported(node, ContextFlags::None);
    if let Some(contextual_type) = contextual_type
        && contextual_type != t
    {
        validate_types(state, node, t, contextual_type);
    }
}

/// `validateSpread`: each spread property against the contextual type's property of the same name.
fn validate_spread(state: &mut TransformState<'_>, node: P<Node>) {
    let expression = node.expression().expect("a spread assignment has an expression");
    let t = state.get_type(expression);
    let Some(contextual_type) = state.checker.get_contextual_type_exported(expression, ContextFlags::None) else {
        return;
    };

    for &property in state.checker.get_properties_of_type_exported(t) {
        let name = property.name();
        let Some(base_property_type) = state.checker.get_type_of_property_of_type_exported(t, name) else { continue };
        let Some(assignment_property_type) = state.checker.get_type_of_property_of_type_exported(contextual_type, name)
        else {
            continue;
        };
        validate_types(state, node, base_property_type, assignment_property_type);
    }
}

/// `validateMethodAssignment`: a member assigned where the other calling convention (method or function) is
/// expected.
///
/// rbxts-rs: only the object literal members so far; class elements (`validateHeritageClause`) come with the classes
/// area.
pub fn validate_method_assignment(state: &mut TransformState<'_>, node: P<Node>) {
    if tsrs_ast::is_class_element(node) && node.parent().is_some_and(tsrs_ast::is_class_like) && node.name().is_some() {
        // rbxts-rs: `validateHeritageClause` for each super type node, with the classes area
    } else if is_object_literal_element_like(node) {
        if node.kind() == Kind::SpreadAssignment {
            if node.expression().is_some_and(|e| e.kind() != Kind::ObjectLiteralExpression) {
                validate_spread(state, node);
            }
        } else {
            validate_object_literal_element(state, node);
        }
    }
}
