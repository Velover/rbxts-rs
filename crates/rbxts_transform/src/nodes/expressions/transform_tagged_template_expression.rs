//! `nodes/expressions/transformTaggedTemplateExpression.ts`

use rbxts_luau::{Expr, create};
use tsrs_ast::{Kind, Node};
use tsrs_core::P;

use crate::classes::prereqs::Prereqs;
use crate::classes::transform_state::TransformState;
use crate::nodes::expressions::transform_expression::transform_expression;
use crate::util::convert_to_indexable_expression::convert_to_indexable_expression;
use crate::util::ensure_transform_order::ensure_transform_order;

/// `` tag`a${b}c` ``: `tag({ "a", "c" }, b)`, the strings cooked.
pub fn transform_tagged_template_expression(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    node: P<Node>,
) -> Expr {
    let tagged = node.as_tagged_template_expression();
    let tag_exp = transform_expression(state, prereqs, tagged.tag);
    let template = tagged.template;

    if template.kind() == Kind::TemplateExpression {
        let template = template.as_template_expression();
        let spans = template.template_spans.nodes();
        let mut strings = vec![create::string(template.head.text())];
        strings.extend(spans.iter().map(|span| create::string(span.as_template_span().literal.text())));

        let expressions: Vec<P<Node>> = spans.iter().map(|span| span.as_template_span().expression).collect();
        let expressions = ensure_transform_order(state, prereqs, &expressions);

        let mut args = vec![create::array(strings)];
        args.extend(expressions);
        create::call(convert_to_indexable_expression(tag_exp), args)
    } else {
        create::call(
            convert_to_indexable_expression(tag_exp),
            vec![create::array(vec![create::string(template.text())])],
        )
    }
}
