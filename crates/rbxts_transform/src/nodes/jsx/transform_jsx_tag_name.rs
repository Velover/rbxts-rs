//! `nodes/jsx/transformJsxTagName.ts`

use rbxts_luau::{Expr, create};
use tsrs_ast::{Kind, Node};
use tsrs_core::P;

use crate::classes::prereqs::Prereqs;
use crate::classes::transform_state::TransformState;
use crate::nodes::expressions::transform_expression::transform_expression;
use crate::util::convert_to_indexable_expression::convert_to_indexable_expression;

/// JavaScript's `c === c.toLowerCase()` on the first UTF-16 unit of `text`.
fn starts_with_lower_case(text: &str) -> bool {
    let Some(first) = text.chars().next() else { return true };
    // rbxts-rs: TypeScript reads one UTF-16 unit; half of a surrogate pair lower-cases to itself
    if first.len_utf16() > 1 {
        return true;
    }
    let mut lower = first.to_lowercase();
    lower.next() == Some(first) && lower.next().is_none()
}

fn transform_jsx_tag_name_expression(state: &mut TransformState<'_>, prereqs: &mut Prereqs, node: P<Node>) -> Expr {
    // host component
    if node.kind() == Kind::Identifier && starts_with_lower_case(node.text()) {
        return create::string(node.text());
    }

    match node.kind() {
        Kind::PropertyAccessExpression => {
            let access = node.as_property_access_expression();
            assert_eq!(access.name.kind(), Kind::Identifier);
            create::property(
                convert_to_indexable_expression(transform_expression(state, prereqs, access.expression)),
                access.name.text(),
            )
        }
        Kind::JsxNamespacedName => create::string(node.text()),
        _ => transform_expression(state, prereqs, node),
    }
}

/// `transformJsxTagName`: a string for host components (`frame`, `ns:name`), else the component's expression.
pub fn transform_jsx_tag_name(state: &mut TransformState<'_>, tag_name: P<Node>) -> Expr {
    let mut expression_prereqs = Prereqs::new();
    let expression = transform_jsx_tag_name_expression(state, &mut expression_prereqs, tag_name);
    // JSX tag names only allow identifiers, this, dotted names, and namespaced names
    assert!(expression_prereqs.statements.is_empty());
    expression
}
