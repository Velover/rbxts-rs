//! The `Flamework` plugin (DESIGN §10.1): the factory, the per-checker instance and the per-file part.
//!
//! - [`Flamework`]: `setup` ([`crate::setup`]) builds the read-only [`FwShared`]; `finish` ([`crate::finish`])
//!   assigns the new ids in program order, resolves the late ids, expands the globs and queues the include JSON and
//!   `flamework.build`.
//! - [`FwInstance`], one per checker: the class hook ([`crate::class_hook`]) and the call hook for user macros
//!   ([`crate::macros`]), with this checker's metadata and decorator caches.
//! - [`FwFilePart`], one per file: the ids it asked for first, its glob and path uses, and the macro sites it left to
//!   later milestones. Only parts cross threads, so the output is the same for any checker count (DESIGN §10.4).
//!
//! Clean builds only: the persistent state is empty and every build is a clean one (TODO(S2): incremental and watch
//! builds reuse the id table, salt and seed, and refuse changed inputs).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use rbxts_plugin::ts::{Node, P, Symbol};
use rbxts_plugin::{
    CallSite, CheckerInfo, Claim, ClassFilter, ClassSite, Cx, Fatal, FileIndex, FinishCx, Interest,
    LuauExpr, LuauStmt, PResult, Plugin, PluginInfo, PluginInstance, SetupCx, SymbolRole, Tag,
};

use crate::build_info::{GlobUse, PathUse};
use crate::ids::NewId;
use crate::metadata::NodeMetadata;
use crate::order::OrderKey;
pub use crate::setup::FwShared;

/// The tag of a call whose callee is a user macro (`@metadata macro`).
pub const TAG_MACRO: Tag = Tag(1);

/// The tsconfig `plugins` entry that turns Flamework on.
pub const TSCONFIG_TRANSFORM: &str = "@flamework-experimental/transformer";

/// Flamework's transformer, compiled in.
#[derive(Debug, Default)]
pub struct Flamework {}

impl Flamework {
    pub fn new() -> Flamework {
        Flamework {}
    }
}

/// A glob or path use, in the order the file's transform met them (they key `metadata.globs` and
/// `metadata.paths` in `flamework.build` in first-use order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathEvent {
    /// A `pathglob` intrinsic: the glob relative to the package root, and where it was written.
    Glob(GlobUse),
    /// A `path` intrinsic.
    Path(PathUse),
}

/// A macro call site left to a later milestone, declined (roblox-ts compiles it as a plain call), or class metadata
/// left out. Reported once per build, at the first site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unported {
    /// The callee and what it needs (`Dependency: a Modding.Generic<T, "dependency"> parameter`).
    pub what: String,
    /// The milestone that brings it (`F2` …).
    pub milestone: &'static str,
    pub pos: rbxts_plugin::Utf16Pos,
}

/// What one file contributes to `finish` (DESIGN §10.1). Owned data only: no handles, no `FileIndex`.
#[derive(Debug, Default, Clone)]
pub struct FwFilePart {
    /// The file relative to the package root (`getFileId`).
    pub file_id: String,
    /// Ids the file asked for that the build info did not have, in first-request order within the file.
    pub new_ids: Vec<NewId>,
    /// The late keys the file used (internal ids), for the late-string contract.
    pub late_keys: Vec<String>,
    pub events: Vec<PathEvent>,
    pub unported: Vec<Unported>,
}

/// The per-file state of an instance, reset in `begin_file`. Per file only: a cache across files would make the
/// parts depend on which checker saw a file first.
#[derive(Debug, Default)]
pub(crate) struct FileState {
    pub part: FwFilePart,
    pub late: HashSet<String>,
    /// Every new id asked for, with the TS transformer's order of the request and the request's sequence number;
    /// sorted and deduplicated into `part.new_ids` by `end_file`.
    pub id_requests: Vec<(OrderKey, usize, NewId)>,
    /// Glob and path uses, likewise sorted into `part.events`.
    pub events: Vec<(OrderKey, usize, PathEvent)>,
    /// The order key of whatever is asking for ids now (the class hook's class).
    pub current_key: OrderKey,
    pub seq: usize,
}

impl FileState {
    /// The next sequence number.
    pub(crate) fn next(&mut self) -> usize {
        self.seq += 1;
        self.seq
    }
}

/// One checker's instance.
pub struct FwInstance {
    pub(crate) shared: Arc<FwShared>,
    /// `NodeMetadata.fromCache`: per checker, by node.
    pub(crate) metadata: RefCell<HashMap<P<Node>, Rc<NodeMetadata>>>,
    /// `getFlameworkDecorators`: per checker, by class declaration.
    pub(crate) decorators: RefCell<HashMap<P<Node>, crate::class_hook::DecoratorKinds>>,
    /// `isUserMacro`, keyed by symbol: kept by the compiler's classification memo too, this one serves the class
    /// hook.
    pub(crate) macro_symbols: RefCell<HashMap<P<Symbol>, bool>>,
    pub(crate) file: RefCell<FileState>,
}

impl FwInstance {
    /// The metadata of a node, cached on this checker.
    pub(crate) fn metadata(&self, cx: &mut Cx<'_>, node: P<Node>) -> Rc<NodeMetadata> {
        if let Some(found) = self.metadata.borrow().get(&node) {
            return found.clone();
        }
        let parsed = Rc::new(NodeMetadata::parse(cx, node));
        self.metadata.borrow_mut().insert(node, parsed.clone());
        parsed
    }

    /// `isUserMacro`: a declaration of the symbol requests `macro`.
    pub(crate) fn is_user_macro(&self, cx: &mut Cx<'_>, symbol: P<Symbol>) -> bool {
        if let Some(&known) = self.macro_symbols.borrow().get(&symbol) {
            return known;
        }
        let is_macro = symbol
            .declarations()
            .iter()
            .any(|&declaration| self.metadata(cx, declaration).is_requested("macro"));
        self.macro_symbols.borrow_mut().insert(symbol, is_macro);
        is_macro
    }

    /// Records a site left to a later milestone.
    pub(crate) fn unported(&self, cx: &Cx<'_>, node: P<Node>, what: String, milestone: &'static str) {
        let pos = cx.utf16_pos(node);
        self.file.borrow_mut().part.unported.push(Unported { what, milestone, pos });
    }
}

impl Plugin for Flamework {
    type Shared = FwShared;
    type State = ();
    type FilePart = FwFilePart;
    type Instance = FwInstance;

    fn info(&self) -> PluginInfo {
        PluginInfo {
            name: "flamework",
            version: crate::VERSION,
            tsconfig_transform: Some(TSCONFIG_TRANSFORM),
        }
    }

    fn setup(&self, cx: &mut SetupCx<'_>, _state: &mut ()) -> Result<Arc<FwShared>, Fatal> {
        crate::setup::setup(cx).map(Arc::new)
    }

    fn instance(&self, shared: Arc<FwShared>, _checker: CheckerInfo) -> FwInstance {
        FwInstance {
            shared,
            metadata: RefCell::default(),
            decorators: RefCell::default(),
            macro_symbols: RefCell::default(),
            file: RefCell::default(),
        }
    }

    fn finish(
        &self,
        shared: &FwShared,
        _state: &mut (),
        parts: &[(FileIndex, &FwFilePart)],
        cx: &mut FinishCx<'_>,
    ) -> Result<(), Fatal> {
        crate::finish::finish(shared, parts, cx)
    }
}

impl PluginInstance for FwInstance {
    type FilePart = FwFilePart;

    fn interest(&self, interest: &mut Interest) {
        interest.calls_by_symbol();
        // Metadata is decided by JSDoc, not by a decorator, so every class is offered.
        interest.classes(ClassFilter::All);
    }

    fn classify_symbol(&self, cx: &mut Cx<'_>, symbol: P<Symbol>, role: SymbolRole) -> Option<Tag> {
        (role == SymbolRole::Callee && self.is_user_macro(cx, symbol)).then_some(TAG_MACRO)
    }

    fn begin_file(&self, cx: &mut Cx<'_>) -> PResult<()> {
        let file_id = self.shared.file_id(&cx.file().path);
        *self.file.borrow_mut() = FileState {
            part: FwFilePart { file_id, ..FwFilePart::default() },
            ..FileState::default()
        };
        Ok(())
    }

    fn call(&self, cx: &mut Cx<'_>, call: &CallSite, tag: Tag) -> PResult<Claim<LuauExpr>> {
        if tag != TAG_MACRO {
            return Ok(Claim::Decline);
        }
        crate::macros::user_macro(self, cx, call)
    }

    fn class(&self, cx: &mut Cx<'_>, class: &ClassSite) -> PResult<Vec<LuauStmt>> {
        crate::class_hook::class_metadata(self, cx, class)
    }

    fn end_file(&self, _cx: &mut Cx<'_>) -> PResult<FwFilePart> {
        let mut file = std::mem::take(&mut *self.file.borrow_mut());
        file.id_requests.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        let mut seen = HashSet::new();
        for (_, _, new) in file.id_requests {
            if seen.insert(new.internal_id.clone()) {
                file.part.new_ids.push(new);
            }
        }
        file.events.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        file.part.events = file.events.into_iter().map(|(_, _, event)| event).collect();
        Ok(file.part)
    }
}
