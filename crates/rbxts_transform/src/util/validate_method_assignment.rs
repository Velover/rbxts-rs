//! `util/validateMethodAssignment.ts` (the expression part; object literal and class element checks come with
//! their node areas).

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
