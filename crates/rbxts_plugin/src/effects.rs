//! roblox-ts's evaluation-order analysis over emitted Luau (`util/evaluation/effects.ts`), the compiler's own,
//! re-exported: what a plugin needs to order evaluations as roblox-ts does (`ensureTransformOrder`,
//! `createOrderedPropertyCall`: when an earlier value must be bound to a local because a later one has effects).
//!
//! - [`effects_of`] (`getEffects`) and [`effects_of_statements`]: what evaluating an expression, or running
//!   statements (prereqs), may do: read or write bindings or the heap, throw, allocate;
//! - [`effects_commute`] (`effectsCommute`): whether two evaluations can be reordered, errors included;
//! - [`join_effects`] (`joinEffects`): both;
//! - [`is_late_read`] (`isLateRead`): Luau reads the expression (a local) only at the consuming instruction.
//!
//! The answers read the facts the compiler records on the Luau it built (bindings, constants, call effects), so they
//! are exact for values from `Cx::compile_*` and conservative for a plugin's own trees (an unknown call has every
//! effect). The JSX variants are not here: plugins never lower JSX.
//!
//! These are the compiler's functions, not copies, so a plugin's choices stay roblox-ts's as the analysis evolves.

pub use rbxts_luau::evaluation::effects::{
    effects_commute, get_effects as effects_of, get_effects_of_statements as effects_of_statements, is_late_read,
    join_effects,
};
pub use rbxts_luau::{Binding, Effects};

#[cfg(test)]
mod tests {
    use rbxts_luau::create;

    use super::*;

    #[test]
    fn the_compiler_s_analysis_orders_a_plugin_s_values() {
        // an unknown call may read and write anything; a literal does nothing; a local is read late
        let call = effects_of(&create::call(create::id("f"), Vec::new()));
        let literal = effects_of(&create::number(1.0));
        let local = effects_of(&create::id("x"));
        assert!(call.throws && call.writes_heap);
        assert!(effects_commute(&literal, &call));
        assert!(!effects_commute(&local, &call), "the call may write `x`");
        assert!(!effects_commute(&call, &call));
        let both = join_effects(literal.clone(), &local);
        assert!(both.reads.contains(&Binding::Name("x".into())));
        assert!(is_late_read(&create::id("x")) && !is_late_read(&create::property(create::id("x"), "y")));
        let statements = vec![create::call_statement(create::call(create::id("g"), Vec::new()))];
        assert!(effects_of_statements(&statements).throws);
    }
}
