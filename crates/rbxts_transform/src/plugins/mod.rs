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
//! claim). Calls are offered:
//! - a call in a property/element/call chain whose callee is reached through plain accesses only (`f()`,
//!   `ns.f()`, `a.b.m()`) is offered before anything of the chain is compiled, so a plugin that takes it leaves the
//!   callee uncompiled (no diagnostics from it, no use of its import);
//! - any other chain call (after a `?.` link or another call) is offered with its receiver or callee already
//!   compiled (and nil-checked when it is inside an optional chain); `compile_receiver` returns a receiver. That
//!   includes `f?.()`, offered as `CallKind::Call` with `receiver: None` once the chain lowering has nil-checked
//!   `f` (`compile_receiver` on it is an internal error: it has no receiver);
//! - `new` expressions before constructor macros; decorators (`@D`, `@D(…)`) before the decorator expression is
//!   compiled.
//!
//! `compile_expr(call.node)` inside the call's own hook lowers the call as roblox-ts does (its macros included),
//! without asking the plugins again, and on the base already compiled, so a receiver is evaluated once. A second
//! compile of the hook's own node is an internal error (it would be lowered twice); a statement hook cannot compile
//! its own statement.
//!
//! **Offered once per offer kind.** The kinds are calls (with `new` and decorators), the expression and statement
//! fallbacks, and the class hook. A node is marked for its kind while its offer runs: compiling it then (a hook's
//! own node through `compile_expr`, `compile_in_order` or an `Arg::Original` slot) does not offer it to that kind
//! again (a call goes through `compile_claimed_call`, on the base already compiled). Other kinds still see it: an
//! expression-kind hook that compiles its own call lets the call plugins claim that call, and a call one expression
//! hook declined cleanly is then offered to the call plugins. Compiling an ancestor of the hook's own node offers
//! and lowers every other node inside it again: don't.
//!
//! **Declining.** Offer state lives in a stack of hook frames ([`file_state::HookFrame`]), one per running hook,
//! so a hook nested in another (through its `compile_*` calls) never resets the outer one's. A claim hook cannot
//! decline its node after it lowered it (`compile_expr` of the node, `compile_call`), compiled the receiver inside
//! a capture, or left anything behind in the scope it writes into or the file: prereqs, a hoist, a root insertion,
//! a header statement, a plugin import, a late string, or an import kept alive (`mark_import_used`), its own or a
//! nested hook's (`request_import_elision` is harmless and allowed). Nor after any hook ran inside it (a nested node
//! offered or claimed, a class hook): that hook would run again. The one exception is a `compile_receiver` into
//! the offer's own scope that is the first thing to grow it: what it left (statements and the hooks it ran) is
//! kept for the next plugin, or roblox-ts, to reuse. A decline that breaks the rule is an internal error, and the
//! offer ends there: no other plugin, and not roblox-ts, gets the node. The rule makes plugins interact: plugin B's
//! interest in a node nested in A's (even one B declines cleanly) makes A's compile-then-decline an error, so a
//! plugin must decide from the AST and types before compiling anything.
//!
//! `RBXTS_PLUGIN_CLAIMS=all` asks the other matching plugins of call, `new` and decorator offers too (not expression
//! or statement hooks), in a scratch scope. A build with it either writes the same output as without it or fails:
//! one where such a plugin changed compiler state before declining (an approval by `is_usable_local_name`
//! included) fails.
//!
//! **Names.** Temporary identifiers plugins ask for (`temp_id`, `hoist`, `hoist_spillable`, `push_to_var`, the
//! `import_named` fallback) are named so that no identifier of the file, no name `is_usable_local_name` approved,
//! and no plugin import can capture them (a hoist or import is declared at the top of the file and may be used in
//! any nested function); approved names and import names avoid the seam temporaries made before them in turn.
//! roblox-ts's own temporaries keep its naming.
//!
//! Not offered yet (`// rbxts-rs: S2`): optional method calls `o.m?.()` / `o[k]?.()` and `super.m()` calls; a
//! plugin whose interest matches one gets an internal error naming it. The `property_key` / `assignment` hooks are
//! not dispatched: registering `accesses_by_marker` or `assignments_by_receiver_symbol` fails the build.

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
