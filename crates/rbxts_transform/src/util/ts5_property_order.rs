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
//! references to generic ones), intersections, type literals, instantiations of anonymous types (in their target's
//! order), homomorphic mapped types and object literal types. For a spread it follows `checkObjectLiteral`: only
//! spreadable members, union operands as `tryMergeUnionOfObjectTypeAndEmptyObject` and `mapType` treat them, and a
//! generic operand's intersection, whose object parts it orders one by one. Elsewhere it keeps TypeScript 7's order:
//! - mapped types over a union of keys (`Record<K, V>`, `Pick`, `{ [P in K]: ... }`): TypeScript 5 follows the
//!   union's type ids there, which no declaration gives (Flamework's serializer sorts these runs itself);
//! - properties an `as` clause renames (``as `get${K}` ``): TypeScript 5 lists them in `T`'s order but keeps no link
//!   to the property they came from, so they go after the others (a filtering `as` clause keeps the link, and its
//!   properties are placed);
//! - unions, tuples.

use tsrs_ast::{Kind, ModifierFlags, Node, Symbol, SymbolFlags};
use tsrs_checker::{
    Checker, MappedType, ObjectFlags, Type, TypeFlags, get_declaration_modifier_flags_from_symbol_exported,
};
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

/// `getSpreadType`'s order for two non-generic object types: the right side's names, then the left side's it lacks.
fn spread(left: Vec<&'static str>, right: Vec<&'static str>) -> Vec<&'static str> {
    let mut names = Vec::with_capacity(right.len() + left.len());
    for name in right.into_iter().chain(left) {
        push_unique(&mut names, name);
    }
    names
}

/// One side of a spread: a non-generic object type's names (spreadable ones only, in order), or a generic type.
enum Part {
    Object(Vec<&'static str>),
    Generic,
}

/// `getSpreadType(left, right)` on the spread so far: empty (`emptyObjectType`), one object type, or an
/// intersection of object types and generic types (5.9.3:67526-67536: a generic side makes an intersection; a
/// non-generic right side after it is spread into the intersection's last part when that part is not generic).
fn spread_onto(state: &mut Vec<Part>, right: Part) {
    match right {
        Part::Generic => {
            // `isEmptyObjectType(left)`: the generic side alone.
            if state.iter().all(|p| matches!(p, Part::Object(names) if names.is_empty())) {
                state.clear();
            }
            state.push(Part::Generic);
        }
        Part::Object(right) => match state.last_mut() {
            Some(Part::Object(left)) => *left = spread(std::mem::take(left), right),
            _ => state.push(Part::Object(right)),
        },
    }
}

/// `isSpreadableProperty` (5.9.3:67555) and not private or protected (`getSpreadType` and
/// `getAnonymousPartialType` skip those): a class's methods, accessors and `#private` members are not spread.
fn is_spread(property: P<Symbol>) -> bool {
    let declarations = property.declarations();
    let in_class = |d: &P<Node>| d.parent().is_some_and(tsrs_ast::is_class_like);
    if declarations.iter().any(|d| in_class(d) && d.name().is_some_and(|n| n.kind() == Kind::PrivateIdentifier)) {
        return false;
    }
    if property.flags().intersects(SymbolFlags::Method | SymbolFlags::GetAccessor | SymbolFlags::SetAccessor)
        && declarations.iter().any(in_class)
    {
        return false;
    }
    !get_declaration_modifier_flags_from_symbol_exported(property)
        .intersects(ModifierFlags::Private | ModifierFlags::Protected)
}

/// `isGenericObjectType`: a type variable, a generic mapped type, or an intersection with one.
fn is_generic(checker: &mut Checker, t: P<Type>) -> bool {
    let flags = t.flags();
    if flags.intersects(TypeFlags::Instantiable) {
        return true;
    }
    if flags.intersects(TypeFlags::Intersection) {
        return t.types().iter().any(|&part| is_generic(checker, part));
    }
    if flags.intersects(TypeFlags::Object) && t.object_flags().intersects(ObjectFlags::Mapped) {
        MappedType::resolve_components(checker, t);
        return t.as_mapped_type().constraint_type().is_some_and(|c| c.flags().intersects(TypeFlags::Instantiable));
    }
    false
}

/// `isEmptyObjectTypeOrSpreadsIntoEmptyObject` (5.9.3:67435): a union part a spread drops.
fn spreads_into_empty(checker: &mut Checker, t: P<Type>) -> bool {
    if t.flags().intersects(
        TypeFlags::Null
            | TypeFlags::Undefined
            | TypeFlags::BooleanLike
            | TypeFlags::NumberLike
            | TypeFlags::BigIntLike
            | TypeFlags::StringLike
            | TypeFlags::EnumLike
            | TypeFlags::NonPrimitive
            | TypeFlags::Index,
    ) {
        return true;
    }
    t.flags().intersects(TypeFlags::Object)
        && checker.get_properties_of_type_exported(t).is_empty()
        && checker.get_index_infos_of_type_exported(t).is_empty()
        && !checker.type_has_call_or_construct_signatures_exported(t)
}

/// The union member `t` was spread from: TypeScript 5 spreads each member (`mapType`), so `t` is one member of a
/// union of object literal types. The member all of whose spread names `t` has, sharing the most declarations with
/// `t`'s properties.
fn spread_member(checker: &mut Checker, members: &[P<Type>], t: P<Type>) -> Option<P<Type>> {
    let own = checker.get_properties_of_type_exported(t);
    let mut best: Option<(usize, P<Type>)> = None;
    let mut candidates = 0;
    for &member in members {
        let properties: Vec<P<Symbol>> =
            checker.get_properties_of_type_exported(member).iter().copied().filter(|&p| is_spread(p)).collect();
        if !properties.iter().all(|p| own.iter().any(|q| q.name() == p.name())) {
            continue;
        }
        candidates += 1;
        let shared = properties
            .iter()
            .filter(|p| {
                own.iter().any(|q| q.name() == p.name() && q.declarations().iter().any(|d| p.declarations().contains(d)))
            })
            .count();
        if best.is_none_or(|(score, _)| shared > score) {
            best = Some((shared, member));
        }
    }
    match best {
        Some((score, member)) if score > 0 || candidates == 1 => Some(member),
        _ => None,
    }
}

/// A spread operand as a [`Part`]: `Some(None)` when it spreads nothing (a primitive, `{}`, a union of those),
/// `None` when the order cannot be rebuilt (TypeScript 7's then).
fn operand_part(
    checker: &mut Checker,
    t: P<Type>,
    mut operand: P<Type>,
    stack: &mut Vec<P<Type>>,
) -> Option<Option<Part>> {
    if is_generic(checker, operand) {
        return Some(Some(Part::Generic));
    }
    if operand.flags().intersects(TypeFlags::Union) {
        // `tryMergeUnionOfObjectTypeAndEmptyObject` (5.9.3:67437): the parts that spread into nothing go; one object
        // left spreads alone (as a partial, in its order); several are spread one by one (`mapType`), and `t` is
        // what one of them gave.
        let parts: Vec<P<Type>> =
            operand.types().iter().copied().filter(|&p| !spreads_into_empty(checker, p)).collect();
        operand = match parts.as_slice() {
            [] => return Some(None),
            [only] => *only,
            several => spread_member(checker, several, t)?,
        };
        if is_generic(checker, operand) {
            return Some(Some(Part::Generic));
        }
    }
    if spreads_into_empty(checker, operand) {
        return Some(None);
    }
    if !operand.flags().intersects(TypeFlags::Object | TypeFlags::Intersection) {
        return None;
    }
    let properties = checker.get_properties_of_type_exported(operand);
    let spread_names: Vec<&'static str> =
        properties.iter().copied().filter(|&p| is_spread(p)).map(|p| p.name()).collect();
    let names = ts5_names(checker, operand, stack).into_iter().filter(|n| spread_names.contains(n)).collect();
    Some(Some(Part::Object(names)))
}

/// `checkObjectLiteral` (5.9.3:78608): the members since the last spread form an object type in source order (a
/// repeated name keeps its first place); each such run, and each spread operand, is spread onto what came before it
/// (`getSpreadType`). With a generic operand the literal's type is an intersection; `t` is then one of its object
/// parts (the one with `t`'s names). `None` where the order cannot be rebuilt.
fn object_literal_names(
    checker: &mut Checker,
    t: P<Type>,
    literal: P<Node>,
    stack: &mut Vec<P<Type>>,
) -> Option<Vec<&'static str>> {
    let properties = checker.get_properties_of_type_exported(t);
    let mut state: Vec<Part> = Vec::new();
    let mut spreads = false;
    let mut run: Vec<&'static str> = Vec::new();
    for &member in literal.properties() {
        if member.kind() != Kind::SpreadAssignment {
            // The property of `t` this member declares, else the member's own symbol (a member a later spread
            // overrode, or one of another part); a computed name nothing kept is not placed.
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
        spreads = true;
        if !run.is_empty() {
            spread_onto(&mut state, Part::Object(std::mem::take(&mut run)));
        }
        let operand = checker.get_type_at_location(member.expression()?);
        if let Some(part) = operand_part(checker, t, operand, stack)? {
            spread_onto(&mut state, part);
        }
    }
    if !spreads {
        return Some(run);
    }
    if !run.is_empty() {
        spread_onto(&mut state, Part::Object(run));
    }
    // The object part that is `t`: the one with exactly `t`'s names, else the one with most of them.
    let own: Vec<&'static str> = properties.iter().map(|p| p.name()).collect();
    let score = |names: &Vec<&'static str>| {
        let common = names.iter().filter(|n| own.contains(n)).count();
        (common == own.len() && common == names.len(), common)
    };
    state
        .into_iter()
        .filter_map(|part| match part {
            Part::Object(names) => Some(names),
            Part::Generic => None,
        })
        .max_by_key(|names| score(names))
}

/// A homomorphic mapped type (`keyof T` constraint) in the order of `T`'s apparent type (`resolveMappedTypeMembers`,
/// 5.9.3:63030, which goes through `T`'s properties in order). With an `as` clause a property goes where the
/// property of `T` it shares a declaration with is (TypeScript 5 links them unless the clause renames); a renamed
/// one keeps TypeScript 7's order after them. `None` for a mapped type over a key union: TypeScript 7's order.
fn mapped_names(checker: &mut Checker, t: P<Type>, stack: &mut Vec<P<Type>>) -> Option<Vec<&'static str>> {
    // Resolving the members resolves the modifiers type.
    let properties = checker.get_properties_of_type_exported(t);
    let mapped = t.as_mapped_type();
    let declaration = mapped.declaration.get()?.as_mapped_type_node();
    let constraint = declaration.type_parameter.as_type_parameter_declaration().constraint?;
    if constraint.kind() != Kind::TypeOperator || constraint.as_type_operator_node().operator != Kind::KeyOfKeyword {
        return None;
    }
    let modifiers = mapped.modifiers_type.get()?;
    let apparent = checker.get_apparent_type_exported(modifiers);
    stack.push(t);
    let source_names = ts5_names(checker, apparent, stack);
    stack.pop();
    if declaration.name_type.is_none() {
        return Some(source_names);
    }
    let source = order_by_names(checker.get_properties_of_type_exported(apparent), &source_names);
    let rank = |p: &P<Symbol>| {
        source.iter().position(|s| s.declarations().iter().any(|d| p.declarations().contains(d))).unwrap_or(usize::MAX)
    };
    let mut ranked: Vec<(usize, &'static str)> = properties.iter().map(|p| (rank(p), p.name())).collect();
    ranked.sort_by_key(|&(index, _)| index);
    Some(ranked.into_iter().map(|(_, name)| name).collect())
}
