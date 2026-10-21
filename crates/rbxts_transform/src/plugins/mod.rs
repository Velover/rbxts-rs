//! The compiler side of the plugin seam (DESIGN §9, §11, §14): `rbxts_plugin`'s backend traits implemented over the
//! transform, the dispatch tables built from each instance's `Interest`, and the build lifecycle around them.
//!
//! - [`host`]: [`PluginHost`], one per checker: the instances, the dispatch tables, the `(symbol, role)`
//!   classification memo, the source-fact caches (line maps, JSDoc) and the hook timings.
//! - [`file_state`]: what one file's transform records for plugins (imports, header, root insertions, late keys,
//!   import-elision requests, file parts).
//! - [`backend`]: `CxBackend` over [`TransformState`](crate::classes::transform_state::TransformState), with the
//!   phase table, and the lowering of `compile_call` / `compile_receiver`.
//! - [`dispatch`]: the claim points the transform calls (calls, `new`, decorators, by-kind fallbacks, classes,
//!   files, root statements, imports).
//! - [`lifecycle`]: activation from the tsconfig, `SetupBackend`, `FinishBackend`, the late-string key check, the
//!   commit with atomic writes, and `explain_checker_diagnostic`.
//!
//! **Claim points.** Plugins are asked before roblox-ts's own macros, in registration order, and the first `Take`
//! wins (`RBXTS_PLUGIN_CLAIMS=all` / [`PluginOptions::claims_all`] asks every matching plugin and reports a double
//! claim). A call is offered once:
//! - a call in a property/element/call chain whose callee is reached through plain accesses only (`f()`,
//!   `ns.f()`, `a.b.m()`) is offered before anything of the chain is compiled, so a plugin that takes it leaves the
//!   callee uncompiled (no diagnostics from it, no use of its import);
//! - any other chain call (after a `?.` link or another call) is offered with its receiver already compiled and,
//!   inside an optional chain, nil-checked: `compile_receiver` returns that value;
//! - `new` expressions before constructor macros; decorators (`@D`, `@D(…)`) before the decorator expression is
//!   compiled.
//!
//! Not offered yet (`// rbxts-rs: S2`): optional calls `f?.()` / `o.m?.()`, `super.m()` calls, and the
//! `property_key` / `assignment` hooks (their interests are accepted and ignored).

pub mod backend;
pub mod dispatch;
pub mod file_state;
pub mod host;
pub mod lifecycle;

mod facts;

pub use host::PluginHost;
pub use lifecycle::{BuildInput, PluginOptions, PluginTimings};

/// The hooks a plugin has, for timings and messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HookKind {
    Setup,
    Classify,
    BeginFile,
    Call,
    Class,
    Expression,
    Statement,
    EndFile,
    Explain,
    Finish,
}

impl HookKind {
    /// Every hook kind, in report order.
    pub const ALL: [HookKind; 10] = [
        HookKind::Setup,
        HookKind::Classify,
        HookKind::BeginFile,
        HookKind::Call,
        HookKind::Class,
        HookKind::Expression,
        HookKind::Statement,
        HookKind::EndFile,
        HookKind::Explain,
        HookKind::Finish,
    ];

    /// The hook's name as plugins know it.
    pub fn label(self) -> &'static str {
        match self {
            HookKind::Setup => "setup",
            HookKind::Classify => "classify_symbol",
            HookKind::BeginFile => "begin_file",
            HookKind::Call => "call",
            HookKind::Class => "class",
            HookKind::Expression => "expression",
            HookKind::Statement => "statement",
            HookKind::EndFile => "end_file",
            HookKind::Explain => "explain_checker_diagnostic",
            HookKind::Finish => "finish",
        }
    }

    pub(crate) fn index(self) -> usize {
        HookKind::ALL.iter().position(|&k| k == self).expect("a hook kind")
    }
}
