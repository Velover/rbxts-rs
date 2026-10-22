//! A test stand-in for F2's `Modding.Generic<T, "guard">`: Flamework with `Flamework.createGuard<T>()` calls taken
//! here, so the guard generator can be checked against TS5 output on real call sites before the user-macro port
//! lands. Not for builds: once F2 takes `Generic<T, "guard">` parameters (see [`super::guard_for_with_dedup`]), this
//! goes.
//!
//! It is the [`Flamework`] plugin (same name, setup, file parts and finish) whose call hook first takes a
//! `createGuard` call from `@flamework-experimental/core` with its `meta` argument left out, and compiles it as
//! `Flamework.createGuard(<guard>)`, as the TS does (the generated argument in place of the missing one).

use std::sync::Arc;

use rbxts_plugin::ts::{P, Type};
use rbxts_plugin::{
    Arg, CallSite, Callee, CheckerInfo, Claim, ClassSite, Cx, Fatal, FileIndex, FinishCx, Interest, LuauExpr,
    LuauStmt, PResult, Plugin, PluginInfo, PluginInstance, SetupCx, SymbolRole, Tag,
};

use crate::packages::CORE_PACKAGE;
use crate::{Flamework, FwFilePart, FwInstance, FwShared};

/// [`Flamework`], with `createGuard<T>()` calls built by the guard generator.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct GuardProbe {
    inner: Flamework,
}

impl GuardProbe {
    pub fn new() -> GuardProbe {
        GuardProbe { inner: Flamework::new() }
    }
}

#[doc(hidden)]
pub struct GuardProbeInstance {
    inner: FwInstance,
}

impl Plugin for GuardProbe {
    type Shared = FwShared;
    type State = ();
    type FilePart = FwFilePart;
    type Instance = GuardProbeInstance;

    fn info(&self) -> PluginInfo {
        self.inner.info()
    }

    fn setup(&self, cx: &mut SetupCx<'_>, state: &mut ()) -> Result<Arc<FwShared>, Fatal> {
        self.inner.setup(cx, state)
    }

    fn instance(&self, shared: Arc<FwShared>, checker: CheckerInfo) -> GuardProbeInstance {
        GuardProbeInstance { inner: self.inner.instance(shared, checker) }
    }

    fn finish(
        &self,
        shared: &FwShared,
        state: &mut (),
        parts: &[(FileIndex, &FwFilePart)],
        cx: &mut FinishCx<'_>,
    ) -> Result<(), Fatal> {
        self.inner.finish(shared, state, parts, cx)
    }
}

impl PluginInstance for GuardProbeInstance {
    type FilePart = FwFilePart;

    fn interest(&self, interest: &mut Interest) {
        self.inner.interest(interest);
    }

    fn classify_symbol(&self, cx: &mut Cx<'_>, symbol: P<rbxts_plugin::ts::Symbol>, role: SymbolRole) -> Option<Tag> {
        self.inner.classify_symbol(cx, symbol, role)
    }

    fn begin_file(&self, cx: &mut Cx<'_>) -> PResult<()> {
        self.inner.begin_file(cx)
    }

    fn call(&self, cx: &mut Cx<'_>, call: &CallSite, tag: Tag) -> PResult<Claim<LuauExpr>> {
        if let Some(target) = create_guard_target(cx, call) {
            let guard = super::guard_for_with_dedup(cx, self.inner.shared.guard_config(), call.node, target)?;
            return cx.compile_call(call, Callee::Original, vec![Arg::Luau(guard)]).map(Claim::Take);
        }
        self.inner.call(cx, call, tag)
    }

    fn class(&self, cx: &mut Cx<'_>, class: &ClassSite) -> PResult<Vec<LuauStmt>> {
        self.inner.class(cx, class)
    }

    fn end_file(&self, cx: &mut Cx<'_>) -> PResult<FwFilePart> {
        self.inner.end_file(cx)
    }
}

/// `T` of a `Flamework.createGuard<T>()` call from core whose `meta` argument is left out: the first element of
/// the parameter type's `_flamework_macro_generic` tuple, as the TS reads a `Generic<T, "guard">`.
fn create_guard_target(cx: &mut Cx<'_>, call: &CallSite) -> Option<P<Type>> {
    let symbol = call.symbol?;
    if symbol.name() != "createGuard" || !call.args.is_empty() {
        return None;
    }
    if cx.declaring_package(symbol)?.name.as_deref() != Some(CORE_PACKAGE) {
        return None;
    }
    let signature = cx.checker().get_resolved_signature_exported(call.node);
    let parameter = cx.checker().get_type_at_position_exported(signature, 0);
    let parameter = cx.checker().get_non_nullable_type(parameter);
    let generic = cx.checker().get_type_of_property_of_type_exported(parameter, "_flamework_macro_generic")?;
    cx.checker().get_type_of_property_of_type_exported(generic, "0")
}
