//! `nodes/expressions/transformBinaryExpression.ts`

use rbxts_luau::{BinaryOperator, Expr, create};
use tsrs_ast::{Kind, Node};
use tsrs_core::P;

use crate::classes::prereqs::Prereqs;
use crate::classes::transform_state::TransformState;
use crate::nodes::transform_logical::transform_logical;
use crate::nodes::transform_logical_or_coalescing_assignment_expression::transform_logical_or_coalescing_assignment_expression;
use crate::nodes::transform_writable::transform_writable_assignment;
use crate::shared::diagnostics::errors;
use crate::util::convert_to_indexable_expression::convert_to_indexable_expression;
use crate::util::create_binary_from_operator::{
    create_assignment_expression, create_binary_from_operator, create_bitwise_from_operator,
    create_compound_assignment_expression, get_assignable_value, get_simple_assignment_operator, is_bitwise_operator,
};
use crate::util::ensure_transform_order::ensure_transform_order;
use crate::util::types::{is_definitely_type, is_number_type, is_possibly_type, is_roblox_type, is_string_type};
use crate::util::validate_not_any::validate_not_any_type;

/// `transformBinaryExpression`
pub fn transform_binary_expression(state: &mut TransformState<'_>, prereqs: &mut Prereqs, node: P<Node>) -> Expr {
    let binary = node.as_binary_expression();
    let operator_kind = binary.operator_token.kind();
    let (left_node, right_node) = (binary.left, binary.right());

    validate_not_any_type(state, left_node);
    validate_not_any_type(state, right_node);

    // banned
    if operator_kind == Kind::EqualsEqualsToken {
        state.add_diagnostic(errors::no_equals_equals(node));
        return Expr::None;
    } else if operator_kind == Kind::ExclamationEqualsToken {
        state.add_diagnostic(errors::no_exclamation_equals(node));
        return Expr::None;
    }

    // logical
    if matches!(operator_kind, Kind::AmpersandAmpersandToken | Kind::BarBarToken | Kind::QuestionQuestionToken) {
        return transform_logical(state, prereqs, node);
    }

    if tsrs_ast::is_logical_or_coalescing_assignment_expression(node) {
        return transform_logical_or_coalescing_assignment_expression(state, prereqs, node);
    }

    if tsrs_ast::is_assignment_operator(operator_kind) {
        // in destructuring, the right-hand side must be executed first
        if left_node.kind() == Kind::ArrayLiteralExpression {
            return state.unsupported_subtree(node, "array destructuring assignment");
        } else if left_node.kind() == Kind::ObjectLiteralExpression {
            return state.unsupported_subtree(node, "object destructuring assignment");
        }

        let writable_type = state.get_type(left_node);
        let value_type = state.get_type(right_node);
        let operator = get_simple_assignment_operator(state, writable_type, operator_kind, value_type);
        let assignment = transform_writable_assignment(
            state,
            prereqs,
            left_node,
            right_node,
            true,
            operator != Some(rbxts_luau::AssignmentOperator::Assign),
        );
        return match operator {
            Some(operator) => {
                let value = get_assignable_value(state, operator, assignment.value, value_type);
                let readable = (!assignment.readable_is_writable).then_some(assignment.readable);
                create_assignment_expression(prereqs, assignment.writable, operator, value, readable)
            }
            None => create_compound_assignment_expression(
                state,
                prereqs,
                assignment.writable,
                writable_type,
                assignment.readable,
                operator_kind,
                assignment.value,
                value_type,
            ),
        };
    }

    if is_bitwise_operator(operator_kind) {
        return create_bitwise_from_operator(state, prereqs, operator_kind, node);
    }

    let mut ordered = ensure_transform_order(state, prereqs, &[left_node, right_node]).into_iter();
    let left = ordered.next().expect("left");
    let right = ordered.next().expect("right");

    if operator_kind == Kind::InKeyword {
        return create::binary(
            create::computed_index(convert_to_indexable_expression(right), left),
            BinaryOperator::Ne,
            Expr::Nil,
        );
    } else if operator_kind == Kind::InstanceOfKeyword {
        let right_type = state.get_type(right_node);
        if is_possibly_type(state, right_type, &[&is_roblox_type]) {
            state.add_diagnostic(errors::no_roblox_symbol_instanceof(right_node));
        }
        let callee = state.ts(node, "instanceof");
        return create::call(callee, vec![left, right]);
    }

    let left_type = state.get_type(left_node);
    let right_type = state.get_type(right_node);

    if matches!(
        operator_kind,
        Kind::LessThanToken | Kind::LessThanEqualsToken | Kind::GreaterThanToken | Kind::GreaterThanEqualsToken
    ) {
        let left_ok = is_definitely_type(state, left_type, &[&is_string_type])
            || is_definitely_type(state, left_type, &[&is_number_type]);
        // roblox-ts checks the left type's number-ness twice here; kept as written
        let right_ok = is_definitely_type(state, right_type, &[&is_string_type])
            || is_definitely_type(state, left_type, &[&is_number_type]);
        if !left_ok || !right_ok {
            state.add_diagnostic(errors::no_non_number_string_relation_operator(node));
        }
    }

    create_binary_from_operator(state, prereqs, left, left_type, operator_kind, right, right_type)
}
