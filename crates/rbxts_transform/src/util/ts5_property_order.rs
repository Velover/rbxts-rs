//! rbxts-rs: a type's properties in the order TypeScript 5's `checker.getPropertiesOfType` lists them. roblox-ts has
//! no such file; plugins need it (Flamework's guards and serializer lay out fields in this order, so it is a wire
//! format).
//!
//! TypeScript 7 lists the same properties in another order for some types:
//! - TypeScript 5 (`resolveObjectTypeMembers` and `getNamedMembers`, `typescript.js` 5.9.3:62316 and 54692) keeps
//!   an insertion-ordered table: the type's own members as bound and merged, then each base type's list in turn,
//!   skipping the names it already has (`addInheritedMembers`, 62107). An inherited member therefore comes in its
//!   base's order, recursively.
//! - TypeScript 7 (`getNamedMembers`, typescript-go `internal/checker/checker.go`, tsrs `checker_11.rs:1172`) sorts
//!   by declaration (file index, then position): for a class or interface the members it takes for its own first
//!   (typescript-go PR #1987, which fixed issue #1968), then the rest. So `C extends B extends A`, with `B`
//!   redeclaring a member of `A`, lists `A`'s members before `B`'s, and `interface X extends Second, First` lists
//!   `First`'s before `Second`'s.
//! - TypeScript 7's `isDeclarationContainedBy` (checker.go, tsrs `checker_11.rs:1220`) compares a member's range with
//!   the container's declarations' ranges without comparing their files. An inherited member declared in another
//!   file at offsets inside one of the container's declarations is taken for an own member and sorted into the first
//!   group.
//! - TypeScript 5 lists a symbol's late-bound members (computed names: a const string, a unique symbol) after its
//!   bound ones (`getResolvedMembersOrExportsOfSymbol`, 62232), type literals included; TypeScript 7's sort puts them
//!   where they are declared.
//! - A spread (`getSpreadType`, 67479) lists the right operand's properties, then the left's that the right lacks;
//!   TypeScript 7 sorts the result by declaration.
//! - A homomorphic mapped type (`{ [P in keyof T]: ... }`, `resolveMappedTypeMembers`, 63030) follows `T`'s list,
//!   which TypeScript 7 sorts.
//!
//! [`ts5_property_order`] rebuilds TypeScript 5's order, recursively, for class and interface types (declared, or
//! references to generic ones), intersections, type literals, object literal types (spreads included), instantiations
//! of anonymous types (in their target's order) and homomorphic mapped types without an `as` clause. Elsewhere it
//! keeps TypeScript 7's order:
//! - mapped types over a union of keys (`Record<K, V>`, `Pick`, `{ [P in K]: ... }`): TypeScript 5 follows the
//!   union's type ids there, which no declaration gives (Flamework's serializer sorts these runs itself);
//! - mapped types with an `as` clause, object literals spreading a generic operand (TypeScript 5 makes an
//!   intersection there), unions, tuples.

use tsrs_ast::{Kind, Node, Symbol, SymbolFlags};
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

/// `isReservedMemberName`: an internal name (`__call`, a computed name's placeholder), not a property's.
fn is_reserved_member_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() >= 2 && b[0] == 0x7f && b[1] != b'@' && b[1] != b'#'
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
    let object_flags = t.object_flags();
    if object_flags.intersects(ObjectFlags::Mapped) {
        return mapped_names(checker, t, stack).unwrap_or_else(|| ts7(checker));
    }
    if object_flags.intersects(ObjectFlags::Anonymous) {
        return anonymous_names(checker, t, stack).unwrap_or_else(|| ts7(checker));
    }
    let target = if object_flags.intersects(ObjectFlags::Reference) { t.object_target().unwrap_or(t) } else { t };
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

/// The names of the members declared by the class, interface or type literal `symbol` (its declarations merged), in
/// TypeScript 5's `getMembersOfSymbol` order: the bound members in their table's insertion order (binding order,
/// merged declarations' members after the first's), then late-bound ones (computed names) by declaration.
fn own_names(checker: &mut Checker, t: P<Type>, symbol: P<Symbol>) -> Vec<&'static str> {
    let mut names = Vec::new();
    if let Some(members) = symbol.members() {
        for i in 0..members.len() {
            let (name, member) = members.entry(i);
            if member.flags().intersects(SymbolFlags::Value) && !is_reserved_member_name(name) {
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

/// Anonymous object types: an instantiation in its target's order, a type literal as declared
/// (`resolveAnonymousTypeMembers` takes `getMembersOfSymbol`), an object literal by its spreads. `None`: TypeScript
/// 7's order.
fn anonymous_names(checker: &mut Checker, t: P<Type>, stack: &mut Vec<P<Type>>) -> Option<Vec<&'static str>> {
    if let Some(target) = t.object_target() {
        stack.push(t);
        let names = ts5_names(checker, target, stack);
        stack.pop();
        return Some(names);
    }
    let symbol = t.symbol()?;
    if symbol.flags().intersects(SymbolFlags::TypeLiteral) {
        return Some(own_names(checker, t, symbol));
    }
    let literal = symbol.declarations().first().copied()?;
    if !symbol.flags().intersects(SymbolFlags::ObjectLiteral) || literal.kind() != Kind::ObjectLiteralExpression {
        return None;
    }
    stack.push(t);
    let names = object_literal_names(checker, t, literal, stack);
    stack.pop();
    names
}

/// `getSpreadType`'s order: the right side's names, then the left side's it lacks.
fn spread(left: Option<Vec<&'static str>>, right: Vec<&'static str>) -> Vec<&'static str> {
    let mut names = Vec::with_capacity(right.len());
    for name in right.into_iter().chain(left.into_iter().flatten()) {
        push_unique(&mut names, name);
    }
    names
}

/// `checkObjectLiteral` (5.9.3:78608): the members since the last spread form an object type in source order (a
/// repeated name keeps its first place); each such run, and each spread operand, is spread onto what came before it
/// (`getSpreadType`). `None` for a generic operand (TypeScript 5 makes an intersection then, not this type) and for
/// a union of several object types.
fn object_literal_names(
    checker: &mut Checker,
    t: P<Type>,
    literal: P<Node>,
    stack: &mut Vec<P<Type>>,
) -> Option<Vec<&'static str>> {
    let properties = checker.get_properties_of_type_exported(t);
    let mut result: Option<Vec<&'static str>> = None;
    let mut run: Vec<&'static str> = Vec::new();
    for &member in literal.properties() {
        if member.kind() != Kind::SpreadAssignment {
            // The property of `t` this member declares, else the member's own symbol (a member a later spread
            // overrode); a computed name nothing kept is not placed.
            let name = properties
                .iter()
                .find(|p| p.declarations().contains(&member))
                .map(|p| p.name())
                .or_else(|| member.symbol().map(|s| s.name()).filter(|n| !is_reserved_member_name(n)));
            if let Some(name) = name {
                push_unique(&mut run, name);
            }
            continue;
        }
        if !run.is_empty() {
            result = Some(spread(result, std::mem::take(&mut run)));
        }
        let mut operand = checker.get_type_at_location(member.expression()?);
        if operand.flags().intersects(TypeFlags::Union) {
            // `T | undefined` spreads as `T` (the nullish parts spread nothing).
            let mut parts =
                operand.types().iter().filter(|p| !p.flags().intersects(TypeFlags::Nullable | TypeFlags::Void));
            match (parts.next(), parts.next()) {
                (Some(&only), None) => operand = only,
                _ => return None,
            }
        }
        if operand.flags().intersects(TypeFlags::Instantiable) {
            return None;
        }
        if !operand.flags().intersects(TypeFlags::Object | TypeFlags::Intersection) {
            // A primitive spreads nothing (`getSpreadType` returns the left side).
            continue;
        }
        let names = ts5_names(checker, operand, stack);
        result = Some(spread(result, names));
    }
    Some(match result {
        None => run,
        Some(left) if run.is_empty() => left,
        Some(left) => spread(Some(left), run),
    })
}

/// A homomorphic mapped type (`keyof T` constraint, no `as` clause) in the order of `T`'s apparent type
/// (`resolveMappedTypeMembers`, 5.9.3:63030). `None` for the others: TypeScript 7's order.
fn mapped_names(checker: &mut Checker, t: P<Type>, stack: &mut Vec<P<Type>>) -> Option<Vec<&'static str>> {
    // Resolving the members resolves the modifiers type.
    checker.get_properties_of_type_exported(t);
    let mapped = t.as_mapped_type();
    let declaration = mapped.declaration.get()?.as_mapped_type_node();
    if declaration.name_type.is_some() {
        return None;
    }
    let constraint = declaration.type_parameter.as_type_parameter_declaration().constraint?;
    if constraint.kind() != Kind::TypeOperator || constraint.as_type_operator_node().operator != Kind::KeyOfKeyword {
        return None;
    }
    let modifiers = mapped.modifiers_type.get()?;
    let apparent = checker.get_apparent_type_exported(modifiers);
    stack.push(t);
    let names = ts5_names(checker, apparent, stack);
    stack.pop();
    Some(names)
}
