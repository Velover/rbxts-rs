//! What one file's transform records for its plugins (in `TransformState::plugin_file`). Empty, and never touched,
//! when the build has no plugins.

use std::cell::OnceCell;

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
}

impl EffectMark {
    /// What grew since `earlier`, as the end of a sentence "declined … after …"; `None` when nothing did.
    pub fn growth_since(&self, earlier: &EffectMark) -> Option<&'static str> {
        if self.prereqs > earlier.prereqs {
            Some("emitting prerequisites (they would run twice)")
        } else if self.root_inserts > earlier.root_inserts {
            Some(
                "leaving statements before the root statement behind (a hoist or an insertion, its own or a nested hook's)",
            )
        } else if self.header > earlier.header {
            Some("leaving header statements behind (its own or a nested hook's)")
        } else if self.imports > earlier.imports {
            Some("leaving a plugin import behind (its own or a nested hook's)")
        } else {
            None
        }
    }
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
    /// Expressions an `expression` hook compiled itself (`compile_expr` of its own node): not offered again.
    pub offered_expressions: FxHashSet<P<Node>>,
    /// `local` names declared at the top level of the main chunk so far (statements done, root insertions,
    /// imports, header).
    pub main_chunk_locals: u32,
    /// What an expression or statement hook did that the node can no longer be declined after (`compile_expr` of
    /// its own node, a hoist, a header, an import): the call path keeps this in its call context.
    pub hook_lowered: Option<&'static str>,
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
        }
    }

    /// The hint for a temporary identifier a plugin asks for (`temp_id`, `hoist`, `push_to_var`, the
    /// `import_named` fallback). The renderer names a temporary `_hint`, `_hint_1`, … against its declaring
    /// scope and the scopes around it only, which is enough for roblox-ts's own temporaries but not for these: a
    /// hoist or an import is declared at the top of the file and may be used in any nested function, where a user
    /// local of that name would capture it. So the hint is changed (`hint2`, `hint3`, …) until no identifier of the
    /// file is `_hint` or `_hint_<n>`.
    pub fn seam_hint(&self, file: P<SourceFile>, hint: &str) -> String {
        let hint = if hint.is_empty() { "seam" } else { hint };
        let names = self.identifier_names(file);
        let taken = |base: &str| {
            let exact = format!("_{base}");
            let numbered = format!("_{base}_");
            names.iter().any(|name| {
                *name == exact
                    || name
                        .strip_prefix(numbered.as_str())
                        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
            })
        };
        if !taken(hint) {
            return hint.to_owned();
        }
        (2..).map(|i| format!("{hint}{i}")).find(|candidate| !taken(candidate)).expect("a free name")
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
