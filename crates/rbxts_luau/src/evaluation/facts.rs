//! `util/evaluation/facts.ts`, the reading half: the facts carried on Luau nodes ([`crate::facts`]) as the effect
//! analysis reads them. Marking them (`markBinding` and the others, which need the TypeScript checker) is the
//! compiler's (`rbxts_transform::util::evaluation::facts`).

use std::sync::Arc;

use crate::{AnyIdentifier, Binding, Effects, Expr};

/// `getCallEffects`
pub fn get_call_effects(expression: &Expr) -> Option<Arc<Effects>> {
    expression.facts()?.call_effects().cloned()
}

/// `isConstantReference`
pub fn is_constant_reference(expression: &Expr) -> bool {
    expression.facts().is_some_and(|f| f.is_constant())
}

/// `isPrimitiveValue`
pub fn is_primitive_value(node: &Expr) -> bool {
    if node.facts().is_some_and(|f| f.is_primitive()) || node.is_simple_primitive() {
        return true;
    }
    if let Expr::Parenthesized(p) = node {
        return is_primitive_value(&p.expression);
    }
    false
}

/// `getBinding`
pub fn get_binding_of_id(identifier: &AnyIdentifier) -> Binding {
    if let Some(key) = identifier.facts().binding() {
        return Binding::Key(key);
    }
    match identifier {
        AnyIdentifier::Identifier(id) => Binding::Name(id.name.clone()),
        AnyIdentifier::Temporary(id) => Binding::Temp(id.id),
    }
}

/// `getBinding` for an identifier expression; `None` for other expressions.
pub fn get_binding(expression: &Expr) -> Option<Binding> {
    match expression {
        Expr::Identifier(id) => Some(match id.facts.binding() {
            Some(key) => Binding::Key(key),
            None => Binding::Name(id.name.clone()),
        }),
        Expr::TemporaryIdentifier(id) => Some(match id.facts.binding() {
            Some(key) => Binding::Key(key),
            None => Binding::Temp(id.id),
        }),
        _ => None,
    }
}

/// `getBuiltinCallEffects`
pub fn get_builtin_call_effects(expression: &Expr) -> Option<Arc<Effects>> {
    expression.facts()?.builtin().cloned()
}

/// `isStableLookup`
pub fn is_stable_lookup(expression: &Expr) -> bool {
    expression.facts().is_some_and(|f| f.is_stable_lookup())
}
