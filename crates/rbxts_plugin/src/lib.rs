//! The plugin API of rbxts-rs (DESIGN §11, with the build lifecycle of §14).
//!
//! A plugin is Rust compiled into the user's build. It has three parts:
//! - a **factory** ([`Plugin`]), one per build tool, `Send + Sync`. It builds the read-only [`Plugin::Shared`]
//!   state in `setup`, owns no mutable state of its own, and merges the file parts in `finish`;
//! - one **instance** ([`PluginInstance`]) per checker and build, made on that checker's thread and never `Send`.
//!   Its hooks run during the transform and see original parse-tree nodes;
//! - one **file part** ([`Plugin::FilePart`]) per transformed file, `Send`, handed back to the factory in program
//!   order. Cross-file state lives only here, so output never depends on the checker count.
//!
//! The compiler implements the services hooks call through the backend traits [`CxBackend`], [`SetupBackend`] and
//! [`FinishBackend`]. Plugins use the wrappers [`Cx`], [`SetupCx`] and [`FinishCx`], which add what does not need
//! the compiler: [`Abort`] values, Luau escaping of late strings, prereq capture.
//!
//! [`erased`] turns a typed plugin into trait objects for the compiler's plugin list (DESIGN §11.6).
//!
//! The crate depends on `rbxts_luau`, `rbxts_project` and tsrs's handle and checker types only, never on the
//! compiler, so the compiler can depend on it.
//!
//! # What the compiler offers, and when
//!
//! **Offer kinds.** There are three: calls (with `new` and decorators, [`PluginInstance::call`]); the expression
//! and statement fallbacks ([`PluginInstance::expression`], [`PluginInstance::statement`]); and the class hook.
//! Expression hooks are asked first, whatever the registration order; within a kind, plugins are asked in
//! registration order and the first `Take` wins. So a call can get an expression-kind offer and then, after a
//! clean decline, a call offer.
//! - A chain's first call reached through plain accesses only (`f()`, `ns.f()`, `a.b.m()`) is offered before
//!   anything of it is compiled.
//! - Any other chain call is offered with its receiver or callee already compiled, and nil-checked only when it is
//!   inside an optional chain. That includes calls after a `?.` link or after another call, such as `g()()` and
//!   `f?.()`.
//! - `new` is offered before constructor macros. Decorators are offered as [`CallKind::Decorator`] before their
//!   expression is compiled; the returned value replaces the decorator expression, which is then applied
//!   (`Class = value(Class) or Class`).
//! - Calls inside a chain (`f()` in `f().m()`) reach expression hooks only when a call plugin compiles the outer
//!   call itself: [`Interest::expressions`] with `CallExpression` does not see every call yet (S2).
//! - Optional method calls (`o.m?.()`, `o[k]?.()`) and `super.m()` are not offered yet (S2). A plugin whose interest
//!   matches one gets an internal error naming it, so an interest (and [`PluginInstance::classify_symbol`]) must
//!   not match symbols it does not mean.
//! - Registering [`Interest::accesses_by_marker`] or [`Interest::assignments_by_receiver_symbol`] fails the build
//!   until S2.
//!
//! **Compiling a hook's own node.** While a node's own offer runs, compiling it does not re-offer it to that kind;
//! the other kinds still see it. An expression hook that compiles its own call lets the call plugins claim that
//! call, and a call hook that compiles its own call asks the expression hooks about it again, so an expression hook
//! can be asked twice about one node: answer the same way both times.
//! - `compile_expr`, `compile_in_order` or an `Arg::Original` slot on the hook's own node lowers it as roblox-ts
//!   does, macros included. A call is lowered on the receiver or callee already compiled, so a single
//!   self-compile evaluates nothing twice. A decorator's own node lowers the decorator's expression.
//! - Lower your own node at most once, by exactly one of `compile_expr` / `compile_in_order` / `Arg::Original` /
//!   `compile_call`. The compiler refuses a second self-compile and a self-compile after `compile_call`, but not
//!   yet `compile_call` twice, `compile_expr` then `compile_call`, or a class hook compiling its own class
//!   expression: those lower it twice, silently.
//! - A statement hook cannot compile its own statement (`compile_*` takes expressions): an internal error naming
//!   the plugin.
//! - Compiling an ancestor of the hook's own node offers and lowers every other node inside that ancestor again.
//!   Don't.
//!
//! **Declining.** A claim hook (`call`, `expression`, `statement`) cannot decline its node after any of these:
//! - it lowered the node (`compile_expr` / `compile_in_order` of the node, `compile_call`);
//! - it compiled the receiver inside a `capture`;
//! - it left anything behind in the scope it writes into or the file, whether its own or a nested hook's:
//!   prereqs, a hoist, a root insertion, a header statement, a plugin import, a late string, or an import kept
//!   alive (`mark_import_used`, or a user import `import_named` reused). `request_import_elision` is harmless and
//!   allowed;
//! - any plugin hook ran inside it, through its `compile_*` calls: a nested node offered (claimed or declined), or
//!   a class hook. That hook would run again.
//!
//! The one exception is a `compile_receiver` into the hook's own scope that is the first thing to grow that scope.
//! What it left (statements, and the hooks it ran) is kept for the next plugin, or roblox-ts, to reuse. An earlier
//! prereq or nested hook voids the exception. A decline that breaks the rule is an internal error naming the
//! plugin. The offer ends there: no other plugin of any kind, and not roblox-ts, gets the node; it becomes `nil`
//! (or an empty statement list), and the build fails.
//!
//! Plugins interact through this rule: plugin B's interest in a node nested in plugin A's node (even one B
//! declines cleanly) turns A's compile-then-decline into an error. So a plugin must decide from the AST and types
//! before compiling anything, as Flamework's TS transformer does.
//!
//! **`RBXTS_PLUGIN_CLAIMS=all`** re-asks the other matching plugins of call, `new` and decorator offers, in a
//! scratch scope; it does not re-ask expression or statement hooks. Two `Take`s are an error. A build with the
//! switch either writes the same output as without it, or fails. It fails when a re-asked plugin changed compiler
//! state before declining (any lowering, placement, import, late string, temporary, diagnostic, or an
//! `is_usable_local_name` approval) or ran a nested hook. A hook's own instance state is out of the compiler's
//! reach, which is one more reason to decide before doing anything.
//!
//! **Placement.** In each file: the `--!` directives, the `-- Compiled with` line, `local TS = require(...)`, plugin
//! imports, the user's own imports, then header statements ([`Cx::header`]).
//!
//! # Build inputs
//!
//! Whatever a plugin's output depends on besides the program is read through the compiler, which records it as a
//! build input and rebuilds what depends on it when it changes (DESIGN §14):
//! - files: [`SetupCx::read_input`];
//! - environment variables: [`SetupCx::env`] in setup, [`Cx::env`] in a file's hooks;
//! - globs: `record_glob` on [`SetupCx`], [`Cx`] and [`FinishCx`], with a [`GlobMatcher`] the compiler can call
//!   again in watch;
//! - other disk reads in `finish`: [`FinishCx::read_dir`], [`FinishCx::exists`], [`FinishCx::read_file`].
//!
//! What a change re-runs follows from where it was read: a setup input re-runs setup and transforms every file; a
//! file's input re-transforms that file (then `finish`); a finish glob re-runs `finish` only.
//!
//! **File parts across processes.** [`Plugin::encode_file_part`] / [`Plugin::decode_file_part`] let the compiler
//! keep a file's part in its incremental state on disk. A plugin that does not implement them (the default) has its
//! cached files transformed again in a new process.

mod cx;
pub mod erased;
mod escape;
mod finish_cx;
mod glob;
mod identifiers;
mod interest;
mod lifecycle;
mod plugin;
mod setup_cx;
mod types;

pub use cx::{Aborted, Cx, CxBackend};
pub use escape::{escape_luau_string, synthesized_string, text_literal};
pub use finish_cx::{DirEntry, EntryKind, FinishBackend, FinishCx};
pub use glob::GlobMatcher;
pub use identifiers::ts5_file_identifiers;
pub use interest::{ClassFilter, Interest, ReceiverInterest};
pub use lifecycle::{BuildKind, ChangeSet, RebuildPlan, StateOrigin};
pub use plugin::{Plugin, PluginInstance};
pub use setup_cx::{SetupBackend, SetupCx};
pub use types::*;

/// A Luau expression (`rbxts_luau::Expr`).
pub type LuauExpr = rbxts_luau::Expr;
/// A Luau statement (`rbxts_luau::Stmt`).
pub type LuauStmt = rbxts_luau::Stmt;
/// A unique local name chosen at render time (`luau.tempId`).
pub type LuauIdent = rbxts_luau::TemporaryIdentifier;
/// A table key in [`ObjectEdits`]: usually a string literal ([`text_literal`]) or a late string.
pub type LuauKey = rbxts_luau::Expr;

/// The handle and checker types hooks receive, re-exported so a plugin needs no direct tsrs dependency for them.
pub mod ts {
    pub use tsrs_ast::{Diagnostic, Kind, Node, SourceFile, Symbol};
    pub use tsrs_checker::{Checker, Signature, Type};
    pub use tsrs_compiler::Program;
    pub use tsrs_core::{CompilerOptions, P};
    pub use tsrs_diagnostics::Category;
}
