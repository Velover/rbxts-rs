//! `util/evaluation/builtins.ts`: Luau libraries and Roblox methods whose lookups are stable.

use std::sync::Arc;

use rbxts_luau::{Effects, Expr};
use tsrs_ast::{Kind, Node};
use tsrs_core::P;

use super::facts::{mark_constant_reference, mark_stable_lookup, set_call_effects};
use crate::classes::transform_state::TransformState;
use crate::util::types::{is_definitely_type, is_in_rbxts_types, is_roblox_type};

const LIBRARIES: &[&str] = &["math", "string", "table", "bit32", "utf8", "coroutine", "task", "os", "debug", "buffer"];
const PURE_MATH: &[&str] = &[
    "abs", "acos", "asin", "atan", "atan2", "ceil", "cos", "cosh", "deg", "exp", "floor", "fmod", "frexp", "ldexp",
    "log", "log10", "max", "min", "modf", "noise", "pow", "rad", "round", "sign", "sin", "sinh", "sqrt", "tan", "tanh",
];

/// `isBuiltinLibrary`: checks the declaration, not the spelling (local lookalikes are ordinary tables).
pub fn is_builtin_library(state: &mut TransformState<'_>, node: P<Node>) -> bool {
    if node.kind() != Kind::Identifier || !LIBRARIES.contains(&node.text()) {
        return false;
    }
    let symbol = state.checker.get_symbol_at_location_exported(node).expect("a library identifier has a symbol");
    let declarations = symbol.declarations();
    !declarations.is_empty()
        && declarations.iter().all(|&d| d.kind() == Kind::ModuleDeclaration && is_in_rbxts_types(state, d))
}

/// `tryMarkBuiltinLibrary`
pub fn try_mark_builtin_library(state: &mut TransformState<'_>, node: P<Node>, expression: &mut Expr) {
    if is_builtin_library(state, node) {
        mark_constant_reference(expression);
    }
}

fn lookup_type(state: &mut TransformState<'_>, expression: P<Node>, optional_chain: bool) -> P<tsrs_checker::Type> {
    let t = state.get_type(expression);
    if optional_chain { state.checker.get_non_nullable_type(t) } else { state.checker.get_non_optional_type(t) }
}

/// `isStableBuiltinMember`: native methods are immutable even when calling them can mutate, throw or yield.
pub fn is_stable_builtin_member(state: &mut TransformState<'_>, node: P<Node>) -> bool {
    let receiver = node.expression().expect("an access has a receiver");
    if is_builtin_library(state, receiver) {
        return true;
    }
    // optional-chain accesses run inside a nil guard; getNonOptionalType alone does not remove a real
    // `T | undefined` union from the original expression
    let optional_chain = tsrs_ast::is_optional_chain(node);
    let t = lookup_type(state, receiver, optional_chain);
    let is_roblox = match state.multi_transform_state.is_roblox_type_cache.get(&t) {
        Some(&cached) => cached,
        None => {
            let result = is_definitely_type(state, t, &[&is_roblox_type]);
            state.multi_transform_state.is_roblox_type_cache.insert(t, result);
            result
        }
    };
    if !is_roblox {
        return false;
    }
    let member_type = lookup_type(state, node, optional_chain);
    let Some(symbol) = member_type.symbol() else { return false };
    let declarations = symbol.declarations();
    !declarations.is_empty()
        && declarations.iter().all(|&d| d.kind() == Kind::MethodSignature && is_in_rbxts_types(state, d))
}

/// `tryMarkBuiltinMember`
pub fn try_mark_builtin_member(state: &mut TransformState<'_>, node: P<Node>, expression: &mut Expr) {
    if !is_stable_builtin_member(state, node) {
        return;
    }
    mark_stable_lookup(expression);
    // these numeric functions are inert for their statically checked arguments; clamp can throw even for numbers,
    // random/randomseed mutate RNG state
    let receiver = node.expression().expect("an access has a receiver");
    if is_builtin_library(state, receiver) && receiver.text() == "math" {
        let name = if node.kind() == Kind::PropertyAccessExpression {
            Some(node.as_property_access_expression().name.text())
        } else {
            let argument = node.as_element_access_expression().argument_expression;
            (argument.kind() == Kind::StringLiteral).then(|| argument.text())
        };
        let nonempty_arguments = node.parent().is_some_and(|call| {
            call.kind() == Kind::CallExpression
                && call.expression() == Some(node)
                && call.arguments().first().is_some_and(|a| a.kind() != Kind::SpreadElement)
        });
        if matches!(name, Some("min" | "max")) && !nonempty_arguments {
            // the types permit empty varargs, which throw. A function value must retain that possibility until an
            // actual call supplies an argument.
            set_call_effects(expression, Arc::new(Effects::may_throw()));
        } else if name.is_some_and(|n| PURE_MATH.contains(&n)) {
            set_call_effects(expression, Arc::new(Effects::none()));
        } else if name == Some("clamp") {
            set_call_effects(expression, Arc::new(Effects::may_throw()));
        }
    }
}
