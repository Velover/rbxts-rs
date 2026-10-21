//! `nodes/binding/transformBindingName.ts`
//!
//! rbxts-rs: the stmts area's minimum (catch clause variables). Binding patterns belong to the binding area and are
//! reported as unsupported here until it lands.

use rbxts_luau::{AnyIdentifier, Stmt};
use tsrs_ast::{Kind, Node};
use tsrs_core::P;

use crate::classes::prereqs::{as_any_identifier, temp_id};
use crate::classes::transform_state::TransformState;
use crate::nodes::expressions::transform_identifier::transform_identifier_defined;

/// `transformBindingName`: the identifier a binding name declares; a pattern binds a temporary and pushes its
/// destructuring to `initializers`.
pub fn transform_binding_name(
    state: &mut TransformState<'_>,
    name: P<Node>,
    _initializers: &mut Vec<Stmt>,
) -> AnyIdentifier {
    if name.kind() == Kind::Identifier {
        as_any_identifier(transform_identifier_defined(state, name))
    } else {
        let id = temp_id("binding");
        state.unsupported_subtree(name, &format!("{:?} variable", name.kind()));
        AnyIdentifier::Temporary(id)
    }
}
