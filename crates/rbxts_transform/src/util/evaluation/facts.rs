//! `util/evaluation/facts.ts`: reading and writing the facts carried on Luau nodes ([`rbxts_luau::facts`]). The
//! reading half lives in [`rbxts_luau::evaluation::facts`] (re-exported here), for plugins too.

pub use rbxts_luau::evaluation::facts::*;

use std::sync::Arc;

use rbxts_luau::{BindingKey, Effects, Expr};
use tsrs_ast::{Kind, NodeFlags, Symbol};
use tsrs_core::P;

/// `markConstantReference`
pub fn mark_constant_reference(expression: &mut Expr) {
    if let Some(facts) = expression.facts_mut() {
        facts.set_constant(true);
    }
}

/// `markStableLookup`: the lookup is inert; evaluating the receiver and invoking the member are not.
pub fn mark_stable_lookup(expression: &mut Expr) {
    if let Some(facts) = expression.facts_mut() {
        facts.set_stable_lookup();
    }
}

/// `markPrimitiveValue`
pub fn mark_primitive_value(expression: &mut Expr) {
    if let Some(facts) = expression.facts_mut() {
        facts.set_primitive();
    }
}

/// `copyValueFacts`
pub fn copy_value_facts(from: &Expr, to: &mut Expr) {
    if let (Some(from), Some(to)) = (from.facts(), to.facts_mut()) {
        to.copy_value_facts(from);
    }
}

/// `setCallEffects`
pub fn set_call_effects(expression: &mut Expr, effects: Arc<Effects>) {
    if let Some(facts) = expression.facts_mut() {
        facts.set_call_effects(effects);
    }
}

/// `markBinding`: the identifier refers to `symbol`'s binding. Imports and function declarations are constant
/// references, and so are `const` variables (other variables are explicitly not).
pub fn mark_binding(identifier: &mut Expr, symbol: P<Symbol>, key: BindingKey) {
    let Some(facts) = identifier.facts_mut() else { return };
    facts.set_binding(key);
    let declarations = symbol.declarations();
    if declarations.iter().any(|d| {
        matches!(
            d.kind(),
            Kind::ImportSpecifier | Kind::ImportClause | Kind::NamespaceImport | Kind::ImportEqualsDeclaration
        )
    }) {
        facts.set_constant(true);
    }
    let declaration = symbol.value_declaration();
    if declaration.is_some_and(|d| d.kind() == Kind::FunctionDeclaration) {
        facts.set_constant(true);
    }
    let mut parent = declaration;
    while let Some(p) = parent
        && matches!(p.kind(), Kind::BindingElement | Kind::ObjectBindingPattern | Kind::ArrayBindingPattern)
    {
        parent = p.parent();
    }
    if let Some(p) = parent
        && p.kind() == Kind::VariableDeclaration
        && let Some(list) = p.parent()
        && list.kind() == Kind::VariableDeclarationList
    {
        facts.set_constant(list.flags().intersects(NodeFlags::Const));
    }
}

