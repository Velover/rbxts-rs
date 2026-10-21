//! `nodes/jsx/transformJsx.ts`

use rbxts_luau::{Expr, create};
use tsrs_ast::Node;
use tsrs_core::P;

use crate::classes::prereqs::Prereqs;
use crate::classes::transform_state::TransformState;
use crate::nodes::jsx::transform_jsx_attributes::transform_jsx_attributes;
use crate::nodes::jsx::transform_jsx_children::transform_jsx_children;
use crate::nodes::jsx::transform_jsx_tag_name::transform_jsx_tag_name;
use crate::nodes::transform_entity_name::transform_entity_name;
use crate::util::convert_to_indexable_expression::convert_to_indexable_expression;
use crate::util::pointer::{MapPointer, create_map_pointer};

/// The JSX factory as an indexable expression (`React.createElement`): the file's `@jsx` pragma, else the
/// `jsxFactory` option, else `React.createElement`. rbxts-rs: the lines `transformJsx` and `transformJsxFragment` share.
pub fn transform_jsx_factory_entity(state: &mut TransformState<'_>, node: P<Node>) -> Expr {
    // jsxFactoryEntity seems to always be defined and will default to `React.createElement`
    let jsx_factory_entity =
        state.resolver.get_jsx_factory_entity(state.checker, node).expect("Expected jsxFactoryEntity to be defined");
    convert_to_indexable_expression(transform_entity_name(state, jsx_factory_entity))
}

/// `transformJsx`: `factory(tag, attributes, ...children)`, with `nil` attributes when there are children but no
/// attributes.
pub fn transform_jsx(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    node: P<Node>,
    tag_name: P<Node>,
    attributes: P<Node>,
    children: &[P<Node>],
) -> Expr {
    let create_element_expression = transform_jsx_factory_entity(state, node);

    let tag_name_exp = transform_jsx_tag_name(state, tag_name);

    let mut attributes_ptr: Option<MapPointer> = None;
    if !attributes.properties().is_empty() {
        let mut ptr = create_map_pointer("attributes");
        transform_jsx_attributes(state, prereqs, attributes, &mut ptr);
        attributes_ptr = Some(ptr);
    }

    let transformed_children = transform_jsx_children(state, prereqs, children);

    let mut args = vec![tag_name_exp];

    if let Some(ptr) = attributes_ptr {
        args.push(ptr.value);
    } else if !transformed_children.is_empty() {
        args.push(create::nil());
    }

    args.extend(transformed_children);

    create::call(create_element_expression, args)
}
