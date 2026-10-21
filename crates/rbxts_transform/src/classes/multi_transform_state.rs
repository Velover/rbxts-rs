//! `classes/MultiTransformState.ts`: state that lives for one compilation, shared by the files of one checker.

use std::rc::Rc;
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_luau::{BindingKey, Effects, Expr};
use tsrs_ast::{Node, SourceFile, Symbol};
use tsrs_checker::Type;
use tsrs_core::P;

use crate::util::evaluation::bindings::BindingUsage;

/// `MultiTransformState`. Type classifications stay valid for the compilation; emitted AST facts can still change.
#[derive(Default)]
pub struct MultiTransformState {
    pub is_primitive_type_cache: FxHashMap<P<Type>, bool>,
    pub is_roblox_type_cache: FxHashMap<P<Type>, bool>,
    pub function_effects: FxHashMap<P<Symbol>, Arc<Effects>>,
    pub binding_keys: FxHashMap<P<Symbol>, BindingKey>,
    pub binding_usage_by_source_file: FxHashMap<P<SourceFile>, Rc<BindingUsage>>,
    pub is_method_cache: FxHashMap<P<Type>, bool>,
    pub is_reported_by_no_unstable_this_type: FxHashSet<P<Node>>,
    pub is_defined_as_let_cache: FxHashMap<P<Symbol>, bool>,
    pub is_any_or_any_array_cache: FxHashMap<P<Type>, bool>,
    pub is_reported_by_no_any_cache: FxHashSet<P<Symbol>>,
    pub is_reported_by_multiple_definitions_cache: FxHashSet<P<Symbol>>,
    pub get_module_exports_cache: FxHashMap<P<Symbol>, Rc<[P<Symbol>]>>,
    pub get_module_exports_alias_map_cache: FxHashMap<P<Symbol>, Rc<FxHashMap<P<Symbol>, Expr>>>,
    /// rbxts-rs: whether a declaration file is under `node_modules/@rbxts/types` (`isRobloxType`, `isBuiltinDeclaration`).
    pub is_rbxts_types_file_cache: FxHashMap<P<SourceFile>, bool>,
}

impl MultiTransformState {
    pub fn new() -> MultiTransformState {
        MultiTransformState::default()
    }
}
