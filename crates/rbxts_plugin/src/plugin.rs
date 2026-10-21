//! The two traits a plugin implements (DESIGN §11.1, §11.2, §14).

use std::sync::Arc;

use tsrs_ast::{Diagnostic, Node, Symbol};
use tsrs_core::P;

use crate::{
    AccessSite, AssignSite, CallSite, ChangeSet, CheckerInfo, Claim, ClassSite, Cx, Fatal,
    FileIndex, FinishCx, Interest, LuauExpr, LuauStmt, PResult, PluginDiagnostic, PluginInfo,
    RebuildPlan, SetupCx, SymbolRole, Tag,
};

/// The factory: one per build tool, shared by every thread. It holds options set in the user's `main`, never
/// mutable build state: that is [`Plugin::Shared`] (read-only, per build), [`Plugin::State`] (persistent, owned
/// by the compiler) and [`Plugin::FilePart`] (per file).
///
/// **Lifecycle** (DESIGN §14). Per build: [`Plugin::rebuild_start`] (watch rebuilds that kept a `Shared`) or
/// [`Plugin::setup`] → [`Plugin::instance`] per checker → the instances' hooks → [`Plugin::finish`]. Then the
/// compiler renders and, only when the build has no errors, commits outputs, artifacts and state in one write
/// phase. A plugin never writes files itself.
///
/// **`Shared` and `State` travel together.** The compiler keeps a `Shared` only together with the state it was
/// made from. A `Shared` made by a build that does not commit (errors or `Fatal`) is dropped, and the next build
/// runs `setup` again on the committed state. (A `Shared` kept from an earlier committed build stays kept when a
/// later build fails: it still matches the committed state.)
pub trait Plugin: Send + Sync + 'static {
    /// Read-only state for the transform: config, env, ids decided up front. Made by `setup`; kept across watch
    /// rebuilds until a build input changes or `rebuild_start` asks for a new one, and only together with the
    /// committed state it was made from: a `Shared` whose build does not commit is dropped (see above).
    type Shared: Send + Sync + 'static;
    /// Persistent state across builds (Flamework: the id table, salt, seed, mode), owned by the compiler.
    ///
    /// The compiler hands a working copy of the last committed state to `setup` and `finish` (a clone, so a
    /// failing build leaves the committed state untouched), and commits the working copy with the build's write
    /// phase. A clean build starts from `Default`. To survive across processes, the plugin persists it itself with
    /// [`FinishCx::write_state_file`] and reloads it in `setup` ([`SetupCx::read_persisted`]).
    ///
    /// It outlives the program it was made with, so it holds **no handles** (`P<…>`, `&'static` program or
    /// source text) **and no [`FileIndex`]**: key per-file data by the file's path (`cx.file().path`).
    type State: Clone + Default + Send + 'static;
    /// What one file contributes to cross-file results. The compiler keeps each file's part from its last
    /// successful transform (by normalised path) and hands the parts of all current files to `finish`.
    ///
    /// Cached parts outlive the program that made them (watch rebuilds free replaced programs, DESIGN §14), so a
    /// part holds **no handles** (`P<Node>`, `P<Symbol>`, `P<Type>`, `&'static` program or source text) **and no
    /// [`FileIndex`]**, which is renumbered every build. Owned strings, numbers and paths only.
    type FilePart: Send + 'static;
    /// The per-checker instance.
    type Instance: PluginInstance<FilePart = Self::FilePart>;

    /// Name, version and activation. Called at registration and when reporting; must be cheap and constant.
    fn info(&self) -> PluginInfo;

    /// Builds the build's `Shared` state. Runs once per build that has no `Shared` to keep, single-threaded,
    /// after the check and before any transform. Read files and env only through `cx` (they become build inputs,
    /// watched and hashed). `state` is the working copy of the persistent state (see [`SetupCx::state_origin`]);
    /// setup may fill it (Flamework creates salt and seed here, in a fixed order).
    fn setup(
        &self,
        cx: &mut SetupCx<'_>,
        state: &mut Self::State,
    ) -> Result<Arc<Self::Shared>, Fatal>;

    /// Called at the start of a watch rebuild, and only when a kept `Shared` exists: the last build committed,
    /// and no build input this plugin read changed (the compiler re-runs `setup` on its own when one did). Never
    /// called for [`crate::BuildKind::Clean`] or [`crate::BuildKind::Incremental`], which always run `setup`.
    /// The plugin may still ask for a new `Shared` ([`RebuildPlan::RerunSetup`], which re-transforms every file).
    /// Default: keep it.
    fn rebuild_start(
        &self,
        shared: &Self::Shared,
        state: &Self::State,
        changes: &ChangeSet,
    ) -> RebuildPlan {
        let _ = (shared, state, changes);
        RebuildPlan::KeepShared
    }

    /// Makes the instance of one checker. Called on that checker's thread, once per checker group and build
    /// (instances never outlive a build, so their caches can key on nodes).
    fn instance(&self, shared: Arc<Self::Shared>, checker: CheckerInfo) -> Self::Instance;

    /// Merges the file parts of every current source file, in program order (`parts` is sorted by this build's
    /// [`FileIndex`]; files that were not transformed this build bring their cached part). Resolves the late
    /// strings of every current file, cached files included ([`FinishCx::resolve_late`]), and queues artifacts.
    /// Runs once per build, after every checker group, on one thread. Not called when the transform reported an
    /// error.
    fn finish(
        &self,
        shared: &Self::Shared,
        state: &mut Self::State,
        parts: &[(FileIndex, &Self::FilePart)],
        cx: &mut FinishCx<'_>,
    ) -> Result<(), Fatal>;
}

/// The per-checker part of a plugin. It holds this checker's types and symbols (caches in `RefCell` / `Cell`), so
/// it is not `Send`: it is made on its checker's thread and stays there.
///
/// **Re-entrancy.** Hooks take `&self`, because `cx.compile_*` dispatches nested nodes, which can reach the same
/// instance again. Never hold a `RefCell` borrow across a `cx.compile_*` call.
///
/// **Ordering** (DESIGN §11.5). Claim hooks (`call`, `property_key`, `assignment`, `expression`, `statement`)
/// ask matching plugins in registration order; the first `Take` wins, and when all decline roblox-ts lowers the
/// node, its own macros included. Broadcast hooks (`begin_file`, `class`, `end_file`) run for every active plugin
/// in registration order.
///
/// **Errors.** `Err(Abort)`: the compiler emits `nil` for the expression or drops the statement and goes on. A
/// hook that returns `Err` without having recorded a new error diagnostic during that call is a plugin bug: the
/// compiler reports an internal error naming the plugin and the file. A panic is caught per file and reported the
/// same way.
///
/// **Phases.** What each hook may call on `Cx` is listed on [`crate::CxBackend`]: the lowering primitives
/// (`compile_*`, prereqs, capture, hoists, root insertions) only in claim hooks and `class`.
pub trait PluginInstance: 'static {
    /// The plugin's [`Plugin::FilePart`].
    type FilePart: Send + 'static;

    /// Registers the nodes this instance wants. Called once, right after the instance is made.
    fn interest(&self, interest: &mut Interest);

    /// Classifies a symbol for the symbol-keyed interests. The compiler memoises the answer per checker and per
    /// `(symbol, role)`: it is asked at most once for each, from inside the first hook dispatch that needs it
    /// (so `cx` is anchored at that site's file; use it for symbol facts only).
    ///
    /// **No diagnostics here.** How often, and on which file, this runs depends on the checker count, so a
    /// warning or error from it would make the diagnostics depend on it too. The compiler reports any diagnostic
    /// from `classify_symbol` as an internal error. Return a tag for the odd case and report it from the hook,
    /// per site. The lowering primitives are refused as well.
    fn classify_symbol(&self, cx: &mut Cx<'_>, symbol: P<Symbol>, role: SymbolRole) -> Option<Tag> {
        let _ = (cx, symbol, role);
        None
    }

    /// Starts a file, before any other hook of the file. Reset per-file caches here. The lowering primitives
    /// are refused; `header`, `import_named` and `late_string` work.
    fn begin_file(&self, cx: &mut Cx<'_>) -> PResult<()> {
        let _ = cx;
        Ok(())
    }

    /// A call, `new` or decorator call that matched a call interest with `tag`. Return the call's Luau value; for
    /// a decorator it replaces the decorator expression, which is then applied (`Class = value(Class) or Class`).
    /// Decide whether to take the call before compiling anything: see "Declining" in the crate docs.
    fn call(&self, cx: &mut Cx<'_>, call: &CallSite, tag: Tag) -> PResult<Claim<LuauExpr>> {
        let _ = (cx, call, tag);
        Ok(Claim::Decline)
    }

    /// A property access that matched an access interest. Return the key to use instead of the written one, as
    /// plain text (not escaped). roblox-ts lowers the access with it in every position (read, write, compound,
    /// `++`, chains): `obj.key` when the key is a valid Luau identifier, else
    /// `obj[text_literal(key)]` ([`crate::text_literal`]). As a method callee, a valid identifier gives
    /// `obj:key(...)`, anything else `obj[text_literal(key)](obj, ...)`.
    fn property_key(
        &self,
        cx: &mut Cx<'_>,
        access: &AccessSite,
        tag: Tag,
    ) -> PResult<Claim<String>> {
        let _ = (cx, access, tag);
        Ok(Claim::Decline)
    }

    /// An assignment, update or `delete` whose target's property symbol was classified with `tag`. Return the
    /// whole expression's Luau value (when `site.value_used`) or an expression to run as a statement.
    fn assignment(&self, cx: &mut Cx<'_>, site: &AssignSite, tag: Tag) -> PResult<Claim<LuauExpr>> {
        let _ = (cx, site, tag);
        Ok(Claim::Decline)
    }

    /// A class the filter admits. The statements go where roblox-ts emits static blocks: after the class is
    /// created, before its decorators are applied. Plugins' statements are concatenated in registration order.
    fn class(&self, cx: &mut Cx<'_>, class: &ClassSite) -> PResult<Vec<LuauStmt>> {
        let _ = (cx, class);
        Ok(Vec::new())
    }

    /// Fallback by syntax kind ([`Interest::expressions`]).
    fn expression(&self, cx: &mut Cx<'_>, node: P<Node>, tag: Tag) -> PResult<Claim<LuauExpr>> {
        let _ = (cx, node, tag);
        Ok(Claim::Decline)
    }

    /// Fallback by syntax kind ([`Interest::statements`]).
    fn statement(&self, cx: &mut Cx<'_>, node: P<Node>, tag: Tag) -> PResult<Claim<Vec<LuauStmt>>> {
        let _ = (cx, node, tag);
        Ok(Claim::Decline)
    }

    /// Ends a file, after every other hook of the file, and returns its part. The lowering primitives are
    /// refused; `header`, `import_named` and `late_string` work.
    fn end_file(&self, cx: &mut Cx<'_>) -> PResult<Self::FilePart>;

    /// Only for files that failed checking (and so are not transformed): may replace a checker diagnostic with a
    /// clearer one. The program, file, checker and source facts, `report` (and `Cx::error` / `warning`),
    /// `is_usable_local_name` and `locals` are available here; the lowering and file-level primitives are refused
    /// (the phase table on [`crate::CxBackend`]).
    fn explain_checker_diagnostic(
        &self,
        cx: &mut Cx<'_>,
        diagnostic: P<Diagnostic>,
    ) -> Option<PluginDiagnostic> {
        let _ = (cx, diagnostic);
        None
    }
}
