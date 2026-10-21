//! `nodes/expressions/transformNewExpression.ts`

use rbxts_luau::{Expr, create};
use tsrs_ast::Node;
use tsrs_core::P;

use crate::classes::prereqs::Prereqs;
use crate::classes::transform_state::TransformState;
use crate::nodes::expressions::transform_expression::transform_expression;
use crate::util::convert_to_indexable_expression::convert_to_indexable_expression;
use crate::util::ensure_transform_order::ensure_transform_order;
use crate::util::types::get_first_construct_symbol;
use crate::util::validate_not_any::validate_not_any_type;

/// `transformNewExpression`: a constructor macro (`new Map()`), or `Class.new(args)`.
pub fn transform_new_expression(state: &mut TransformState<'_>, prereqs: &mut Prereqs, node: P<Node>) -> Expr {
    let class_expression = node.expression().expect("a new expression has an expression");
    validate_not_any_type(state, class_expression);

    if let Some(symbol) = get_first_construct_symbol(state, class_expression)
        && let Some(r#macro) = state.macro_manager.get_constructor_macro(symbol)
    {
        return match r#macro.transform {
            Some(transform) => transform(state, prereqs, node),
            None => state.unsupported(node, &format!("constructor macro {}", r#macro.name)),
        };
    }

    let expression = convert_to_indexable_expression(transform_expression(state, prereqs, class_expression));
    let args = ensure_transform_order(state, prereqs, node.arguments());
    create::call(create::property(expression, "new"), args)
}
