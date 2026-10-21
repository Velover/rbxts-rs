//! `macros/types.ts`: the macro signatures.
//!
//! rbxts-rs: a macro table entry's transform is `None` until its area is ported; the macro manager still resolves
//! every entry's symbol, so a call to an unported macro is reported as unsupported instead of being compiled as an
//! ordinary call.

use rbxts_luau::Expr;
use tsrs_ast::Node;
use tsrs_core::P;

use crate::classes::prereqs::Prereqs;
use crate::classes::transform_state::TransformState;

/// `IdentifierMacro`
pub type IdentifierMacro = fn(&mut TransformState<'_>, P<Node>) -> Expr;

/// `ConstructorMacro`
pub type ConstructorMacro = fn(&mut TransformState<'_>, &mut Prereqs, P<Node>) -> Expr;

/// `CallMacro`. Operands are opaque values: assigning to them or transforming their source again bypasses
/// evaluation planning. Use `push_to_var` only for the macro's own working state.
pub type CallMacro = fn(&mut TransformState<'_>, &mut Prereqs, P<Node>, Expr, Vec<Expr>) -> Expr;

/// `PropertyCallMacro`: the node is a call whose expression is a property or element access.
pub type PropertyCallMacro = fn(&mut TransformState<'_>, &mut Prereqs, P<Node>, Expr, Vec<Expr>) -> Expr;

/// A macro table entry: its name (`Array.push`, `typeIs`) and its transform once ported.
#[derive(Clone, Copy)]
pub struct Macro<F> {
    pub name: &'static str,
    pub transform: Option<F>,
}
