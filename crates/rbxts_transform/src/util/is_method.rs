//! `util/isMethod.ts`: whether calling a value passes a receiver (`self`), decided by its call signatures.

use tsrs_ast::{Kind, Node, Symbol};
use tsrs_checker::{ContextFlags, ElementFlags, Signature, SignatureFlags, SignatureKind, Type, TypeFlags};
use tsrs_core::P;

use crate::classes::transform_state::TransformState;
use crate::shared::diagnostics::errors;
use crate::util::traversal::skip_upwards;
use crate::util::types::{get_type_arguments, walk_types};

fn contains_instantiable_type(t: P<Type>) -> bool {
    if t.flags().intersects(TypeFlags::UnionOrIntersection) {
        return t.types().iter().any(|&t| contains_instantiable_type(t));
    }
    t.flags().intersects(TypeFlags::Instantiable)
}

fn can_change_receiver_convention(state: &mut TransformState<'_>, t: P<Type>) -> bool {
    if !contains_instantiable_type(t) {
        return false;
    }

    // a fixed non-void union member keeps the instantiated union from becoming void
    if t.flags().intersects(TypeFlags::Union)
        && t.types()
            .iter()
            .any(|&m| !m.flags().intersects(TypeFlags::Void | TypeFlags::Never) && !contains_instantiable_type(m))
    {
        return false;
    }

    if t.flags().intersects(TypeFlags::Conditional) {
        let conditional = t.as_conditional_type();
        let root = conditional.root.get().expect("a conditional type has a root");
        let mut check_type = root.check_type.get().expect("a conditional root has a check type");
        let mut extends_type = conditional.extends_type.get().expect("a conditional type has an extends type");
        // singleton tuples suppress distribution without changing the void test
        if check_type.is_tuple_type() && extends_type.is_tuple_type() {
            let check_elements = get_type_arguments(state, check_type);
            let extends_elements = get_type_arguments(state, extends_type);
            if check_elements.len() == 1 && extends_elements.len() == 1 {
                check_type = check_elements[0];
                extends_type = extends_elements[0];
            }
        }

        // a conditional that removes void can still have an unconstrained fallback type
        let node = root.node.get().expect("a conditional root has a node").as_conditional_type_node();
        let true_type = state.checker.get_type_from_type_node_exported(node.true_type);
        let false_type = state.checker.get_type_from_type_node_exported(node.false_type);
        let void_type = state.checker.get_void_type();
        if true_type.flags().intersects(TypeFlags::Never)
            && false_type == check_type
            && state.checker.is_type_assignable_to_exported(void_type, extends_type)
        {
            return false;
        }
    }

    match state.checker.get_base_constraint_of_type_exported(t) {
        None => true,
        Some(constraint) => {
            let void_type = state.checker.get_void_type();
            state.checker.is_type_assignable_to_exported(void_type, constraint)
        }
    }
}

fn is_method_declaration(node: P<Node>) -> bool {
    if matches!(node.kind(), Kind::MethodDeclaration | Kind::MethodSignature) {
        return true;
    }
    // object literal function expressions have an implicit receiver
    node.kind() == Kind::FunctionExpression
        && skip_upwards(node).parent().is_some_and(|p| p.kind() == Kind::PropertyAssignment)
}

// rbxts-rs: `checker.getParameterCount` (`pub(crate)` in tsrs).
fn get_parameter_count(state: &mut TransformState<'_>, signature: P<Signature>) -> usize {
    let parameters = signature.parameters();
    if signature.flags().intersects(SignatureFlags::HasRestParameter) {
        let rest_type = state.checker.get_type_of_symbol_exported(parameters[parameters.len() - 1]);
        if rest_type.is_tuple_type() {
            let target = rest_type.target_tuple_type();
            return parameters.len() + target.fixed_length()
                - usize::from(!target.combined_flags.get().intersects(ElementFlags::Variable));
        }
    }
    parameters.len()
}

// rbxts-rs: `checker.isAritySmaller` (`pub(crate)` in tsrs).
fn is_arity_smaller(state: &mut TransformState<'_>, signature: P<Signature>, target: P<Node>) -> bool {
    let parameters = target.parameters();
    let mut target_parameter_count = parameters
        .iter()
        .take_while(|p| {
            p.initializer().is_none()
                && p.question_token().is_none()
                && p.as_parameter_declaration().dot_dot_dot_token().is_none()
        })
        .count();
    if parameters.first().is_some_and(|&p| tsrs_ast::is_this_parameter(p)) {
        target_parameter_count = target_parameter_count.saturating_sub(1);
    }
    !state.checker.has_effective_rest_parameter_exported(signature)
        && get_parameter_count(state, signature) < target_parameter_count
}

// rbxts-rs: `checker.getContextualCallSignature` (`pub(crate)` in tsrs). Several applicable signatures are
// combined by `getIntersectedSignatures`, which is not public either; they give no signature here.
fn get_contextual_call_signature(state: &mut TransformState<'_>, t: P<Type>, node: P<Node>) -> Option<P<Signature>> {
    let signatures = state.checker.get_signatures_of_type_exported(t, SignatureKind::Call);
    let mut applicable = signatures.iter().copied().filter(|&s| !is_arity_smaller(state, s, node));
    let first = applicable.next()?;
    applicable.next().is_none().then_some(first)
}

// rbxts-rs: `checker.getContextualSignature` (`pub(crate)` in tsrs). A union contextual type whose members give
// different signatures has a contextual signature in TypeScript when the signatures are identical apart from
// `this` and return types; `compareSignaturesIdentical` is not public, so that case gives none here.
fn get_contextual_signature(state: &mut TransformState<'_>, node: P<Node>) -> Option<P<Signature>> {
    let t = state.checker.get_apparent_type_of_contextual_type(node, ContextFlags::Signature)?;
    if !t.flags().intersects(TypeFlags::Union) {
        return get_contextual_call_signature(state, t, node);
    }
    let mut found = None;
    for &member in t.types() {
        if let Some(signature) = get_contextual_call_signature(state, member, node) {
            match found {
                None => found = Some(signature),
                Some(existing) if existing == signature => {}
                Some(_) => return None,
            }
        }
    }
    found
}

/// The `this` parameter TypeScript 5 gives a signature that has none of its own.
// rbxts-rs: TypeScript 5 calls a function expression or object literal method without type parameters and without
// an explicit `this` parameter context sensitive, and so gives it its contextual signature's `this` parameter
// (`assignContextualParameterTypes`). TypeScript 7's `hasContextSensitiveParameters` asks whether such a
// function uses `this`, so one that has only annotated parameters and doesn't use `this` keeps no `this`
// parameter, and a `function () {}` in a `(this: defined) => void` slot would lose its `self`. Emulate TypeScript 5
// in exactly that case; with an unannotated parameter, or `this` used, both agree and the signature has it already.
fn ts5_contextual_this_parameter(state: &mut TransformState<'_>, signature: P<Signature>) -> Option<P<Symbol>> {
    let declaration = signature.declaration()?;
    if !(declaration.kind() == Kind::FunctionExpression || tsrs_ast::is_object_literal_method(declaration))
        || declaration.type_parameter_list().is_some()
        || declaration.parameters().iter().any(|p| p.type_node().is_none())
        || declaration.flags().intersects(tsrs_ast::NodeFlags::ContainsThis)
    {
        return None;
    }
    get_contextual_signature(state, declaration)?.this_parameter()
}

fn is_method_inner(state: &mut TransformState<'_>, node: P<Node>, t: P<Type>) -> bool {
    let mut has_method_definition = false;
    let mut has_callback_definition = false;

    for &signature in state.checker.get_signatures_of_type_exported(t, SignatureKind::Call) {
        let this_parameter = signature.this_parameter().or_else(|| ts5_contextual_this_parameter(state, signature));
        if let Some(this_parameter) = this_parameter {
            let this_type = state
                .checker
                .get_type_of_symbol_at_location(this_parameter, Some(node))
                .unwrap_or_else(|| state.checker.get_error_type());
            if can_change_receiver_convention(state, this_type)
                && state.multi_transform_state.is_reported_by_no_unstable_this_type.insert(node)
            {
                state.add_diagnostic(errors::no_unstable_this_type(node));
            }
            if !this_type.flags().intersects(TypeFlags::Void) {
                has_method_definition = true;
            } else {
                has_callback_definition = true;
            }
        } else if let Some(declaration) = signature.declaration() {
            if is_method_declaration(declaration) {
                has_method_definition = true;
            } else {
                has_callback_definition = true;
            }
        }
    }

    if has_method_definition && has_callback_definition {
        state.add_diagnostic(errors::no_mixed_type_call(node));
    }

    has_method_definition
}

/// `isMethodFromType`
pub fn is_method_from_type(state: &mut TransformState<'_>, node: P<Node>, t: P<Type>) -> bool {
    let mut result = false;
    walk_types(state, t, &mut |state, t| {
        // `result ||= …` skips (and does not cache) the check once a method was found
        if t.symbol().is_some() && !result {
            let value = match state.multi_transform_state.is_method_cache.get(&t) {
                Some(&cached) => cached,
                None => {
                    let value = is_method_inner(state, node, t);
                    state.multi_transform_state.is_method_cache.insert(t, value);
                    value
                }
            };
            result |= value;
        }
    });
    result
}

/// `isMethod`: by the node's type.
pub fn is_method(state: &mut TransformState<'_>, node: P<Node>) -> bool {
    let t = state.get_type(node);
    is_method_from_type(state, node, t)
}
