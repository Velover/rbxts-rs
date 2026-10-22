//! rbxts-rs: a type's properties in the order TypeScript 5's `checker.getPropertiesOfType` lists them. roblox-ts has
//! no such file; plugins need it (Flamework's guards and serializer lay out fields in this order, so it is a wire
//! format).
//!
//! TypeScript 7 lists the same properties in another order for some class and interface types:
//! - TypeScript 5 (`resolveObjectTypeMembers` and `getNamedMembers`, `typescript.js` 5.9.3:62316 and 54692) keeps
//!   an insertion-ordered table: the type's own members as bound and merged, then each base type's list in turn,
//!   skipping the names it already has (`addInheritedMembers`, 62107). An inherited member therefore comes in its
//!   base's order, recursively.
//! - TypeScript 7 (`getNamedMembers`, typescript-go `internal/checker/checker.go`, tsrs `checker_11.rs:1172`) lists
//!   the members it takes for the type's own, then the rest, each group sorted by declaration (file index, then
//!   position). So `C extends B extends A`, with `B` redeclaring a member of `A`, lists `A`'s members before `B`'s,
//!   and `interface X extends Second, First` lists `First`'s before `Second`'s.
//! - TypeScript 7's `isDeclarationContainedBy` (checker.go, tsrs `checker_11.rs:1220`) compares a member's range with
//!   the container's declarations' ranges without comparing their files. An inherited member declared in another
//!   file at offsets inside one of the container's declarations is taken for an own member and sorted into the first
//!   group.
//!
//! [`ts5_property_order`] rebuilds TypeScript 5's order for the cases that follow from declarations: class and
//! interface types (declared, or references to generic ones) and intersections, recursively. Elsewhere (unions,
//! mapped types, object literal and spread types, tuples) it keeps TypeScript 7's order.

use tsrs_ast::{Symbol, SymbolFlags};
use tsrs_checker::{Checker, ObjectFlags, Type, TypeFlags};
use tsrs_core::P;

/// Deeper than any real inheritance chain; past it the order is TypeScript 7's.
const MAX_DEPTH: usize = 64;

/// `checker.getPropertiesOfType(t)` in TypeScript 5's order: the same symbols as
/// [`Checker::get_properties_of_type_exported`], reordered. A property the TypeScript 5 rules do not place keeps its
/// TypeScript 7 position after the placed ones.
pub fn ts5_property_order(checker: &mut Checker, t: P<Type>) -> Vec<P<Symbol>> {
    let properties = checker.get_properties_of_type_exported(t);
    let names = ts5_names(checker, t, &mut Vec::new());
    order_by_names(properties, &names)
}

/// `properties` sorted by their name's index in `names`, the unnamed ones last (stable).
fn order_by_names(properties: &[P<Symbol>], names: &[&'static str]) -> Vec<P<Symbol>> {
    let mut keyed: Vec<(usize, P<Symbol>)> = properties
        .iter()
        .map(|&p| (names.iter().position(|&n| n == p.name()).unwrap_or(usize::MAX), p))
        .collect();
    keyed.sort_by_key(|&(index, _)| index);
    keyed.into_iter().map(|(_, p)| p).collect()
}

fn push_unique(names: &mut Vec<&'static str>, name: &'static str) {
    if !names.contains(&name) {
        names.push(name);
    }
}

/// The property names of `t` in TypeScript 5's order (names that are not properties of `t` may appear too: the
/// caller only uses the positions of its properties' names).
fn ts5_names(checker: &mut Checker, t: P<Type>, stack: &mut Vec<P<Type>>) -> Vec<&'static str> {
    let ts7 = |checker: &mut Checker| checker.get_properties_of_type_exported(t).iter().map(|p| p.name()).collect();
    if stack.contains(&t) || stack.len() >= MAX_DEPTH {
        return ts7(checker);
    }
    let flags = t.flags();
    if flags.intersects(TypeFlags::Intersection) {
        // `getPropertiesOfUnionOrIntersectionType`: each part's list in turn, a name where it first comes.
        stack.push(t);
        let mut names = Vec::new();
        for &part in t.types() {
            for name in ts5_names(checker, part, stack) {
                push_unique(&mut names, name);
            }
        }
        stack.pop();
        return names;
    }
    if !flags.intersects(TypeFlags::Object) {
        return ts7(checker);
    }
    let target = if t.object_flags().intersects(ObjectFlags::Reference) { t.object_target().unwrap_or(t) } else { t };
    if !target.object_flags().intersects(ObjectFlags::ClassOrInterface) {
        return ts7(checker);
    }
    let Some(symbol) = target.symbol() else {
        return ts7(checker);
    };

    stack.push(t);
    let mut names = own_names(checker, t, symbol);
    // `resolveObjectTypeMembers`: `addInheritedMembers` per base type, in the order of the heritage clauses. The
    // instantiated base has the same names as the declared one.
    for &base in checker.get_base_types_exported(target) {
        for name in ts5_names(checker, base, stack) {
            push_unique(&mut names, name);
        }
    }
    stack.pop();
    names
}

/// The names of the members declared by the class or interface `symbol` (its declarations merged), in TypeScript 5's
/// `getMembersOfSymbol` order: the bound members in their table's insertion order (binding order, merged
/// declarations' members after the first's), then late-bound ones (computed names) by declaration.
fn own_names(checker: &mut Checker, t: P<Type>, symbol: P<Symbol>) -> Vec<&'static str> {
    let mut names = Vec::new();
    if let Some(members) = symbol.members() {
        for i in 0..members.len() {
            let (name, member) = members.entry(i);
            if member.flags().intersects(SymbolFlags::Value) {
                push_unique(&mut names, name);
            }
        }
    }
    // Late-bound members: properties of `t` declared directly in one of `symbol`'s declarations, which the bound
    // table does not name.
    let declarations = symbol.declarations();
    let mut late: Vec<(usize, tsrs_core::TextPos, &'static str)> = Vec::new();
    for &property in checker.get_properties_of_type_exported(t) {
        if names.contains(&property.name()) {
            continue;
        }
        let own = property.declarations().iter().find_map(|&d| {
            let parent = d.parent()?;
            declarations.iter().position(|&c| c == parent).map(|index| (index, d.pos()))
        });
        if let Some((index, pos)) = own {
            late.push((index, pos, property.name()));
        }
    }
    late.sort_by_key(|&(index, pos, _)| (index, pos));
    names.extend(late.into_iter().map(|(_, _, name)| name));
    names
}
