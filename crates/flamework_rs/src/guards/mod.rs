//! Guards (F3): `t` checks generated from types, a port of the TS transformer's `util/functions/buildGuardFromType.ts`
//! (with `propertyKey.ts`, `identifierName.ts`, `getInstanceTypeFromType.ts` and the guard parts of
//! `macros/intrinsics/guards.ts`), emitting Luau instead of TypeScript for roblox-ts to compile.
//!
//! **Entry points.** Each builds one guard with a generator of its own, as each TS call does:
//! - [`guard_for`] (`buildGuardFromType`): class metadata (`flamework:guard`, `flamework:parameter_guards`,
//!   `flamework:return_guard`), component instance guards;
//! - [`guard_for_with_dedup`] (`buildGuardFromTypeWithDedup`): `Modding.Generic<T, "guard">` (`createGuard<T>()`).
//!   With `transformer.optimizations.guardGenerationDedupLimit` set, types met that many times are declared once
//!   as locals ahead of the statement (prereqs), or kept with the guard in a function of their own when the scope
//!   has no room for them;
//! - [`guard_fields`] (`buildGuardsFromType`): one guard per property (component attributes);
//! - [`tuple_guards`] (`buildTupleGuardsIntrinsic`): the `tuple-guards` intrinsic, `{ { guards }, rest, { after } }`.
//!
//! **Text.** The Luau is what roblox-ts makes of the TS builder's output, so it matches a TS5 build byte for byte,
//! but for the order of union members: TS7 orders a union's constituents its own way (deterministically), and the
//! guard follows it (DESIGN §10.4, behaviour-neutral). Locals are named as TypeScript's printer names the builder's
//! unique names ([`names`]).
//!
//! **Not the TS's.**
//! - The Luau-limits budget: the TS lets Flamework's code claim 16 locals in a file's main chunk and 64 in a
//!   function, shared by every call site there. Here each dedup guard checks its own count against those budgets
//!   and against the locals the compiler has counted so far ([`rbxts_plugin::Cx::locals`]): a file whose dedup
//!   guards together pass 16 locals keeps them as prereqs where the TS would wrap the later ones in a function.
//! - `Promise<T>` (`Promise.is`, the runtime library's `TS.Promise`): plugins have no runtime-library primitive, so
//!   the guard imports `t` first, which loads the runtime library as every `TS.import` does (the TS leaves the unused
//!   `t` import out).
//! - The guard-library warnings ("Flamework core was not found", "Valid `@rbxts/t` was not found") come once per
//!   guard, at its first use of `t`, where the TS gives one per type it visits.

mod generator;
mod library;
pub(crate) mod names;
pub mod types;

use rbxts_luau::{Expr, Stmt, create};
use rbxts_plugin::ts::{Kind, Node, P, Type};
use rbxts_plugin::{Cx, LuauExpr, PResult};
use tsrs_checker::ElementFlags;

pub use generator::{DEDUP_HEURISTIC_LIMIT, GUARD_RECURSION_LIMIT};
use generator::Generator;
use names::declared;

use crate::setup::FwShared;

/// What the generator needs of the project: the package name (the guard library) and the dedup limit.
#[derive(Clone, Copy, Debug)]
pub struct GuardConfig<'a> {
    /// `package.json`'s name: inside the Flamework packages the guard library is `@rbxts/t` itself.
    pub package_name: &'a str,
    /// `transformer.optimizations.guardGenerationDedupLimit`.
    pub dedup_limit: Option<u32>,
}

impl FwShared {
    /// The guard generator's view of the project.
    pub fn guard_config(&self) -> GuardConfig<'_> {
        GuardConfig { package_name: &self.package_name, dedup_limit: self.config.guard_generation_dedup_limit }
    }
}

/// `buildGuardFromType`: the guard of `ty`. Errors (unsupported types) are reported at `node`, with a note for each
/// type the guard was being built inside of, and abort the hook.
pub fn guard_for(cx: &mut Cx<'_>, config: GuardConfig<'_>, node: P<Node>, ty: P<Type>) -> PResult<LuauExpr> {
    let mut generator = Generator::new(cx, config.package_name, node);
    let mut guard = generator.build_guard(ty)?;
    generator.names.assign(generator.cx, &mut [], Some(&mut guard));
    Ok(guard)
}

/// The TS's `LOCAL_BUDGET` and `FUNCTION_LOCAL_BUDGET` (`luauLimits.ts`), and `COMPACT_LOCALS`.
const FILE_LOCAL_BUDGET: usize = 16;
const FUNCTION_LOCAL_BUDGET: usize = 64;
const COMPACT_LOCALS: usize = 150;

/// `buildGuardFromTypeWithDedup`: the guard of `ty` for a `Generic<T, "guard">` argument at `node` (the call). With a
/// dedup limit, the shared guards go ahead of the current statement as prereqs (`cx.prereq`), or, when the scope has
/// no room for them, into a function with the guard.
pub fn guard_for_with_dedup(cx: &mut Cx<'_>, config: GuardConfig<'_>, node: P<Node>, ty: P<Type>) -> PResult<LuauExpr> {
    let mut generator = Generator::new(cx, config.package_name, node);
    if let Some(limit) = config.dedup_limit {
        generator.calculate_dedup(ty, limit.max(1));
    }
    let guard = generator.build_guard(ty)?;

    let count = generator.dedup.len();
    if count == 0 || dedup_fits(generator.cx, node, count) {
        let mut guard = guard;
        let mut statements: Vec<Stmt> =
            generator.dedup.drain(..).map(|d| create::local(declared(&d.id), Some(d.value))).collect();
        generator.names.assign(generator.cx, &mut statements, Some(&mut guard));
        // TODO(seam-additions): `Cx::insert_before_statement`, where the TS's `prereqList` puts them.
        for statement in statements {
            generator.cx.prereq(statement);
        }
        return Ok(guard);
    }

    let mut guard = generator.hold_in_function(guard);
    generator.names.assign(generator.cx, &mut [], Some(&mut guard));
    Ok(guard)
}

/// Whether `count` more locals fit ahead of the statement at `node` (see the module docs).
fn dedup_fits(cx: &Cx<'_>, node: P<Node>, count: usize) -> bool {
    let budget = if in_function(node) { FUNCTION_LOCAL_BUDGET } else { FILE_LOCAL_BUDGET };
    count <= budget && cx.locals().function as usize + count <= COMPACT_LOCALS
}

/// `enclosingScope(node)` is a function (or a class static block), not the file.
fn in_function(node: P<Node>) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if tsrs_ast::is_function_like(Some(parent)) || parent.kind() == Kind::ClassStaticBlockDeclaration {
            return true;
        }
        current = parent.parent();
    }
    false
}

/// `buildGuardsFromType`: a guard per property of `ty`, keyed for a Luau table: by the property's table key when
/// `interface` (as `t.interface` checks them), else by its name (a component's attributes).
pub fn guard_fields(
    cx: &mut Cx<'_>,
    config: GuardConfig<'_>,
    node: P<Node>,
    ty: P<Type>,
    interface: bool,
) -> PResult<Vec<(LuauExpr, LuauExpr)>> {
    let mut generator = Generator::new(cx, config.package_name, node);
    let fields = generator.build_guards_from_type(ty, interface)?;
    let mut map = create::map(fields);
    generator.names.assign(generator.cx, &mut [], Some(&mut map));
    let Expr::Map(fields) = map else { unreachable!("a map") };
    Ok(fields.into_iter().map(|field| (field.index, field.value)).collect())
}

/// `buildTupleGuardsIntrinsic` (the `tuple-guards` intrinsic): `{ { element guards }, rest guard or nil }`, and a
/// third entry `{ guards after the rest }` for a tuple with elements after its rest. A tuple of only a rest element
/// is an array: `{ {}, element guard }`.
pub fn tuple_guards(cx: &mut Cx<'_>, config: GuardConfig<'_>, node: P<Node>, ty: P<Type>) -> PResult<LuauExpr> {
    if cx.checker().is_array_type(ty) {
        let element = cx.checker().get_type_arguments(ty)[0];
        let guard = guard_for(cx, config, node, element)?;
        return Ok(create::array(vec![create::array(Vec::new()), guard]));
    }
    if !tsrs_checker::is_tuple_type_exported(ty) {
        let text = cx.checker().type_to_string_exported(ty);
        return Err(cx.error(node, format!("Intrinsic encountered non-tuple type: {text}")));
    }

    let arguments = cx.checker().get_type_arguments(ty);
    let infos = ty.target_tuple_type().element_infos();
    let mut guards = Vec::new();
    let mut rest: Option<Expr> = None;
    let mut after = Vec::new();
    for (index, &element) in arguments.iter().enumerate() {
        let info = infos.get(index).copied().unwrap_or_default();
        let guard = guard_for(cx, config, info.labeled_declaration.unwrap_or(node), element)?;
        if info.flags.intersects(ElementFlags::Rest) {
            rest = Some(guard);
        } else if rest.is_some() {
            after.push(guard);
        } else {
            guards.push(guard);
        }
    }

    let mut entries = vec![create::array(guards), rest.unwrap_or_else(create::nil)];
    if !after.is_empty() {
        entries.push(create::array(after));
    }
    Ok(create::array(entries))
}
