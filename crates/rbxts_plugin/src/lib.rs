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

mod cx;
pub mod erased;
mod escape;
mod finish_cx;
mod interest;
mod lifecycle;
mod plugin;
mod setup_cx;
mod types;

pub use cx::{Aborted, Cx, CxBackend};
pub use escape::{escape_luau_string, text_literal};
pub use finish_cx::{FinishBackend, FinishCx};
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
