//! What one file's transform records for its plugins (in `TransformState::plugin_file`). Empty, and never touched,
//! when the build has no plugins.

use std::cell::{OnceCell, RefCell};

use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_luau::{Expr, Stmt};
use rbxts_plugin::FileInfo;
use rbxts_plugin::erased::ErasedPart;
use tsrs_ast::{Kind, Node, SourceFile};
use tsrs_core::P;

/// A top-level statement of the file and where its output sits in the file's statement list.
#[derive(Clone, Copy, Debug)]
pub struct RootRange {
    pub node: P<Node>,
    /// The statement's own Luau (not its leading comments or root insertions).
    pub start: usize,
    pub end: usize,
    /// The diagnostics its transform reported, as indices into the file's diagnostics.
    pub diagnostics_start: usize,
    pub diagnostics_end: usize,
    /// The `TS` uses its transform added.
    pub runtime_lib_uses: u32,
}

/// Elision decisions for one re-transform of an import declaration (see `dispatch::end_file`).
#[derive(Debug, Default)]
pub struct ImportOverrides {
    /// Aliases elided: a plugin removed their value uses and nothing in the output refers to them.
    pub elided: FxHashSet<P<Node>>,
    /// Aliases kept though the checker saw no value use (`mark_import_used`, a reused import).
    pub forced: FxHashSet<P<Node>>,
}

/// One import made by `import_named`.
#[derive(Debug)]
pub struct PluginImport {
    pub specifier: String,
    pub export: String,
    pub value: Expr,
    pub statement: Option<Stmt>,
}

/// The lengths of what a hook, or a hook nested in it, can leave behind besides its value: the prereq scope it
/// writes to, root insertions (hoists), header statements and plugin imports. A declined node must leave the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectMark {
    pub prereqs: usize,
    pub root_inserts: usize,
    pub header: usize,
    pub imports: usize,
    /// Late keys used and imports kept alive (`late_string`, `mark_import_used`).
    pub late_keys: usize,
    pub forced_imports: usize,
}

impl EffectMark {
    /// What grew since `earlier`, as the end of a sentence "declined … after …"; `None` when nothing did.
    pub fn growth_since(&self, earlier: &EffectMark) -> Option<&'static str> {
        if self.prereqs > earlier.prereqs {
            Some("emitting prerequisites (they would run twice)")
        } else if self.root_inserts > earlier.root_inserts {
            Some(
                "leaving statements before the root statement behind (a hoist or an insertion, its own or a nested \
                 hook's)",
            )
        } else if self.header > earlier.header {
            Some("leaving header statements behind (its own or a nested hook's)")
        } else if self.imports > earlier.imports {
            Some("leaving a plugin import behind (its own or a nested hook's)")
        } else if self.late_keys > earlier.late_keys {
            Some("using a late string (its own or a nested hook's)")
        } else if self.forced_imports > earlier.forced_imports {
            Some("keeping an import alive (mark_import_used or a reused import, its own or a nested hook's)")
        } else {
            None
        }
    }
}

/// One running hook on the frame stack ([`PluginFileState::frames`]). Pushed by every hook call and popped after
/// it, so a nested hook (reached through the outer hook's `compile_*` calls) never touches the outer hook's state.
#[derive(Clone, Copy, Debug, Default)]
pub struct HookFrame {
    /// What the hook did that its node can no longer be declined after: `compile_expr` of the node itself,
    /// `compile_call`, a hoist, a root insertion, a header, an import, a receiver compiled inside a capture.
    pub lowered: Option<&'static str>,
    /// `compile_call` ran.
    pub called: bool,
    /// Hooks (claim or class hooks) that ran while this one ran, through its `compile_*` calls: declining after
    /// them would run them again.
    pub nested_hooks: u32,
    /// Of those, the ones the lazy `compile_receiver` ran: the receiver is kept for reuse, so they do not run
    /// again.
    pub receiver_nested_hooks: u32,
    /// The hook called a primitive that changes compiler state (any lowering, placement, import, late string,
    /// temporary, name approval or diagnostic), not only a query. `RBXTS_PLUGIN_CLAIMS=all` relies on it.
    pub touched: bool,
    /// The hook compiled its own node: a second time is an internal error (it would be lowered twice).
    pub self_compiled: bool,
}

/// The per-file plugin state.
#[derive(Default)]
pub struct PluginFileState {
    pub info: Option<FileInfo>,
    /// Per plugin: `begin_file` succeeded (a plugin whose `begin_file` failed gets no claims in the file).
    pub begun: Vec<bool>,
    /// Per plugin: the file part `end_file` returned.
    pub parts: Vec<Option<ErasedPart>>,
    /// The late keys the file's Luau uses: (plugin, namespaced key), in first-use order.
    pub late_keys: Vec<(usize, String)>,
    late_seen: FxHashSet<String>,
    /// `header` statements, in call order.
    pub header: Vec<Stmt>,
    /// `import_named` imports, in request order.
    pub imports: Vec<PluginImport>,
    pub(crate) import_names: FxHashSet<String>,
    identifier_names: OnceCell<FxHashSet<&'static str>>,
    pub(crate) import_cache: FxHashMap<(String, String), Expr>,
    /// Statements for before the current top-level statement (`insert_before_root_statement`, `hoist`).
    pub root_inserts: Vec<Stmt>,
    /// The file's top-level statements, as transformed.
    pub root_ranges: Vec<RootRange>,
    /// `request_import_elision` aliases, in request order.
    pub elision_requests: Vec<P<Node>>,
    /// `mark_import_used` aliases, and user imports `import_named` reused.
    pub forced_imports: FxHashSet<P<Node>>,
    /// Import aliases the compiler itself compiled a reference to.
    pub compiled_aliases: FxHashSet<P<Node>>,
    /// Set while an import declaration is transformed again with elision decisions.
    pub import_overrides: Option<ImportOverrides>,
    /// Calls already offered to the plugins (a chain call asked before its callee was compiled).
    pub offered: FxHashSet<P<Node>>,
    /// Nodes being offered right now, one set per offer kind: a dispatch of one of them while its offer runs (a
    /// hook compiling its own node, or an ancestor of it) is not offered to that kind again. The kinds are
    /// separate, so an expression hook compiling its own call still lets the call plugins claim that call.
    /// Calls, `new` and decorators:
    pub in_offer: FxHashSet<P<Node>>,
    /// The expression and statement fallbacks:
    pub in_fallback_offer: FxHashSet<P<Node>>,
    /// The class hook:
    pub in_class_offer: FxHashSet<P<Node>>,
    /// `local` names declared at the top level of the main chunk so far (statements done, root insertions,
    /// imports, header).
    pub main_chunk_locals: u32,
    /// The hooks running now, innermost last.
    pub frames: Vec<HookFrame>,
    /// Names `is_usable_local_name` approved: the plugin may declare them anywhere, so no seam temporary or
    /// plugin import takes them.
    approved_names: RefCell<FxHashSet<String>>,
    /// The hints seam temporaries were given: their names `_hint` / `_hint_<n>` are taken for later approvals.
    seam_bases: FxHashSet<String>,
}

impl PluginFileState {
    /// The state of a file with `plugins` plugins.
    pub fn new(info: FileInfo, plugins: usize) -> PluginFileState {
        PluginFileState {
            info: Some(info),
            begun: vec![true; plugins],
            parts: (0..plugins).map(|_| None).collect(),
            ..PluginFileState::default()
        }
    }

    /// Every identifier text in the file (made on first use): a name none of them has cannot shadow, or be
    /// shadowed by, anything the file declares or refers to.
    pub fn identifier_names(&self, file: P<SourceFile>) -> &FxHashSet<&'static str> {
        self.identifier_names.get_or_init(|| {
            fn walk(node: P<Node>, out: &mut FxHashSet<&'static str>) {
                if node.kind() == Kind::Identifier {
                    out.insert(node.text());
                }
                node.for_each_child(&mut |child| {
                    walk(child, out);
                    false
                });
            }
            let mut out = FxHashSet::default();
            walk(file.as_node(), &mut out);
            out
        })
    }

    /// The effect lengths now, with the prereq scope at `prereqs` statements.
    pub fn effect_mark(&self, prereqs: usize) -> EffectMark {
        EffectMark {
            prereqs,
            root_inserts: self.root_inserts.len(),
            header: self.header.len(),
            imports: self.imports.len(),
            late_keys: self.late_keys.len(),
            forced_imports: self.forced_imports.len(),
        }
    }

    /// The hint for a temporary identifier a plugin asks for (`temp_id`, `hoist`, `push_to_var`, the
    /// `import_named` fallback). The renderer names a temporary `_hint`, `_hint_1`, … against its declaring
    /// scope and the scopes around it only, which is enough for roblox-ts's own temporaries but not for these: a
    /// hoist or an import is declared at the top of the file and may be used in any nested function, where a user
    /// local of that name would capture it. So the hint is changed (`hint2`, `hint3`, …) until no identifier of the
    /// file is `_hint` or `_hint_<n>`.
    ///
    /// Names `is_usable_local_name` approved and plugin import names count too, so a name the plugin may declare
    /// in a nested scope never captures a seam temporary; the hint chosen is recorded, so a later approval cannot
    /// take its names either.
    pub fn seam_hint(&mut self, file: P<SourceFile>, hint: &str) -> String {
        let hint = if hint.is_empty() { "seam" } else { hint };
        let base = {
            let names = self.identifier_names(file);
            let approved = self.approved_names.borrow();
            let taken = |base: &str| {
                let exact = format!("_{base}");
                let numbered = format!("_{base}_");
                let matches = |name: &str| {
                    name == exact
                        || name
                            .strip_prefix(numbered.as_str())
                            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
                };
                names.iter().any(|n| matches(n))
                    || approved.iter().any(|n| matches(n))
                    || self.import_names.iter().any(|n| matches(n))
            };
            if taken(hint) {
                (2..).map(|i| format!("{hint}{i}")).find(|candidate| !taken(candidate)).expect("a free name")
            } else {
                hint.to_owned()
            }
        };
        self.seam_bases.insert(base.clone());
        base
    }

    /// `name` is a seam temporary's name: `_base` or `_base_<n>` for a recorded hint.
    fn is_seam_name(&self, name: &str) -> bool {
        let Some(rest) = name.strip_prefix('_') else { return false };
        self.seam_bases.iter().any(|base| {
            rest.strip_prefix(base.as_str()).is_some_and(|tail| {
                tail.is_empty()
                    || tail
                        .strip_prefix('_')
                        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            })
        })
    }

    /// The number of names approved so far (an approval changes later naming, so it counts as touching state).
    pub fn approved_count(&self) -> usize {
        self.approved_names.borrow().len()
    }

    /// `name` is used by the file's source, a plugin import, an approved name or a seam temporary.
    pub fn name_taken(&self, file: P<SourceFile>, name: &str) -> bool {
        self.identifier_names(file).contains(name)
            || self.import_names.contains(name)
            || self.approved_names.borrow().contains(name)
            || self.is_seam_name(name)
    }

    /// `is_usable_local_name`: a valid identifier, not a `luau.globals` name, and not taken
    /// ([`PluginFileState::name_taken`]). An approved name is recorded, so later seam temporaries and plugin
    /// imports avoid it.
    pub fn approve_name(&self, file: P<SourceFile>, name: &str) -> bool {
        let usable = rbxts_luau::is_valid_identifier(name)
            && !rbxts_luau::is_reserved_identifier(name)
            && !self.name_taken(file, name);
        if usable {
            self.approved_names.borrow_mut().insert(name.to_owned());
        }
        usable
    }

    /// Records a late key the file's Luau uses.
    pub fn use_late_key(&mut self, plugin: usize, key: String) {
        if self.late_seen.insert(key.clone()) {
            self.late_keys.push((plugin, key));
        }
    }
}

/// The number of `local` names a statement declares in the scope it is in.
pub fn declared_locals(statement: &Stmt) -> u32 {
    match statement {
        Stmt::VariableDeclaration(declaration) => declaration.left.len() as u32,
        Stmt::FunctionDeclaration(declaration) => u32::from(declaration.localize),
        _ => 0,
    }
}
