//! `CxBackend` over the transform state (DESIGN §9, §11.3): one [`HookBackend`] per hook call, anchored at the node
//! being dispatched, with the phase table of `rbxts_plugin::CxBackend` enforced.

use std::rc::Rc;
use std::sync::Arc;

use rbxts_luau::{Effects, Expr, Stmt, create};
use rbxts_plugin::erased::DynInstance;
use rbxts_plugin::{
    Aborted, Arg, BinOp, CallKind, CallSite, Callee, Cx, CxBackend, FileInfo, JsDocTag, LocalCounts, LuauIdent,
    ObjectEdits, PResult, PluginDiagnostic, Utf16Pos,
};
use rbxts_project::{PackageInfo, ProjectServices, ResolvedModuleInfo};
use tsrs_ast::{Diagnostic, DiagnosticExt, Kind, Node, SourceFile, Symbol, new_diagnostic};
use tsrs_checker::Checker;
use tsrs_compiler::Program;
use tsrs_core::{CompilerOptions, P};
use tsrs_diagnostics::{Category, new_ad_hoc_message};

use super::HookKind;
use super::dispatch;
use super::facts;
use super::file_state::{EffectMark, HookFrame, declared_locals};
use super::host::PluginHost;
use crate::classes::prereqs::{Prereqs, temp_id};
use crate::classes::transform_state::TransformState;
use crate::nodes::expressions::transform_call_expression::{
    PropertyKey, create_ordered_property_call, fix_void_arguments_for_roblox_functions,
};
use crate::nodes::expressions::transform_expression::transform_expression;
use crate::nodes::expressions::transform_super_keyword::transform_super_keyword;
use crate::nodes::transform_optional_chain::{ChainItem, transform_chain_item};
use crate::shared::diagnostics::node_range;
use crate::util::add_one_if_array_type::add_one_if_array_type;
use crate::util::convert_to_indexable_expression::convert_to_indexable_expression;
use crate::util::ensure_transform_order::ensure_transform_order;
use crate::util::evaluation::effects::{effects_commute, get_effects, get_effects_of_statements, join_effects};
use crate::util::is_method::{is_method, is_method_from_type};
use crate::util::types::{get_first_construct_symbol, get_first_defined_symbol};
use crate::util::validate_not_any::validate_not_any_type;
use crate::util::value_to_id_str::value_to_id_str;
use crate::util::wrap_return_if_lua_tuple::wrap_return_if_lua_tuple;

/// Which column of the phase table a hook is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Claim hooks and `class`: everything.
    Claim,
    /// `begin_file`, `end_file`: no lowering primitives.
    File,
    /// `classify_symbol`: facts only.
    Classify,
    /// `explain_checker_diagnostic`: facts and reports.
    Explain,
}

impl Phase {
    pub(crate) fn of(hook: HookKind) -> Phase {
        match hook {
            HookKind::Call | HookKind::Class | HookKind::Expression | HookKind::Statement => Phase::Claim,
            HookKind::BeginFile | HookKind::EndFile => Phase::File,
            HookKind::Classify => Phase::Classify,
            HookKind::Explain | HookKind::Setup | HookKind::Finish => Phase::Explain,
        }
    }
}

/// The rows of the phase table that can refuse (`report` is refused only in `classify_symbol`, with its own
/// message).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Lowering,
    FileLevel,
}

/// The syntactic shape of a claimed call, for `compile_receiver` and `compile_call`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CallShape {
    /// `f(…)`: the callee is compiled as a whole.
    Call,
    /// `super(…)`
    Super,
    /// `obj.name(…)`
    Property { access: P<Node>, name: &'static str },
    /// `obj[argument](…)`
    Element { access: P<Node>, argument: P<Node> },
    /// `new C(…)`
    New,
    /// `@D` (`call` is `None`) or `@D(…)`.
    Decorator { call: Option<P<Node>> },
}

/// A call being offered to plugins.
#[derive(Clone)]
pub(crate) struct CallContext {
    pub site: CallSite,
    pub shape: CallShape,
    /// The compiled base: the receiver of a property or element call, else the callee. Given by the chain lowering
    /// (eager), or compiled on first need (lazy).
    pub base: Option<Expr>,
    /// The effect marks right before and after a lazily compiled base, measured against the offer's own prereq
    /// scope: a declining plugin may leave what compiling the base left (the next plugin, or roblox-ts, reuses the
    /// base), but nothing from before it. `None` when the base was not compiled lazily into that scope.
    pub base_marks: Option<(EffectMark, EffectMark)>,
    /// The base was compiled lazily inside a capture: its prereqs are the plugin's, so it is never reused.
    pub base_captured: bool,
    /// The chain link of a chain call, to lower the call on its compiled base.
    pub item: Option<ChainItem>,
}

impl CallContext {
    pub(crate) fn new(site: CallSite, shape: CallShape, base: Option<Expr>) -> CallContext {
        CallContext { site, shape, base, base_marks: None, base_captured: false, item: None }
    }
}

/// One hook call's backend.
pub(crate) struct HookBackend<'s, 'a> {
    pub state: &'s mut TransformState<'a>,
    pub host: &'a PluginHost,
    prereqs: Option<&'s mut Prereqs>,
    captures: Vec<Prereqs>,
    call: Option<&'s mut CallContext>,
    plugin: usize,
    hook: HookKind,
    phase: Phase,
    anchor: P<Node>,
}

/// Runs one hook of `plugin`. `prereqs` is the scope `prereq` / `push_to_var` / `compile_*` write to (claim hooks
/// only); `call` the call being claimed. A hook's `Err` without a new error diagnostic is reported as an internal
/// error naming the plugin; `Err(())` means the hook failed (its error is recorded).
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_hook<R>(
    state: &mut TransformState<'_>,
    prereqs: Option<&mut Prereqs>,
    call: Option<&mut CallContext>,
    plugin: usize,
    hook: HookKind,
    anchor: P<Node>,
    f: impl FnOnce(&dyn DynInstance, &mut Cx<'_>) -> PResult<R>,
) -> Result<R, ()> {
    run_hook_framed(state, prereqs, call, plugin, hook, anchor, f).0
}

/// [`run_hook`], also returning the hook's frame ([`HookFrame`]): what it did that its node can no longer be
/// declined after, and the hooks that ran inside it. The frame is pushed before the hook and popped after it; the
/// enclosing hook's frame counts this hook as nested (`classify_symbol`, memoised, does not count).
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_hook_framed<R>(
    state: &mut TransformState<'_>,
    prereqs: Option<&mut Prereqs>,
    call: Option<&mut CallContext>,
    plugin: usize,
    hook: HookKind,
    anchor: P<Node>,
    f: impl FnOnce(&dyn DynInstance, &mut Cx<'_>) -> PResult<R>,
) -> (Result<R, ()>, HookFrame) {
    let host = state.plugins.expect("the build has plugins");
    let diagnostics_before = state.diagnostics.diagnostics.len();
    if hook != HookKind::Classify
        && let Some(parent) = state.plugin_file.frames.last_mut()
    {
        parent.nested_hooks += 1;
    }
    state.plugin_file.frames.push(HookFrame::default());
    host.enter(Some(plugin), hook);
    let result = {
        let mut backend = HookBackend {
            state: &mut *state,
            host,
            prereqs,
            captures: Vec::new(),
            call,
            plugin,
            hook,
            phase: Phase::of(hook),
            anchor,
        };
        let mut cx = Cx::new(&mut backend);
        f(&*host.plugins[plugin].instance, &mut cx)
    };
    host.exit();
    let frame = state.plugin_file.frames.pop().expect("this hook's frame");
    let result = match result {
        Ok(value) => Ok(value),
        Err(_) => {
            if !has_errors_since(state, diagnostics_before) {
                let message = format!(
                    "plugin `{}` returned Err from {} without reporting an error",
                    host.name(plugin),
                    hook.label()
                );
                state.internal_error(anchor, &message);
            }
            Err(())
        }
    };
    (result, frame)
}

/// Runs `classify_symbol` (no `PResult`; diagnostics are refused there).
pub(crate) fn run_classify(
    state: &mut TransformState<'_>,
    plugin: usize,
    anchor: P<Node>,
    f: impl FnOnce(&dyn DynInstance, &mut Cx<'_>) -> Option<rbxts_plugin::Tag>,
) -> Option<rbxts_plugin::Tag> {
    run_hook(state, None, None, plugin, HookKind::Classify, anchor, |instance, cx| Ok(f(instance, cx))).ok().flatten()
}

pub(crate) fn has_errors_since(state: &TransformState<'_>, index: usize) -> bool {
    state.diagnostics.diagnostics[index..].iter().any(|d| d.category() == Category::Error)
}

/// A plugin diagnostic as a tsrs diagnostic: the node's `getStart()` to its end, the hint on a line of its own,
/// related information attached.
pub(crate) fn to_diagnostic(diagnostic: &PluginDiagnostic) -> P<Diagnostic> {
    let at = |node: P<Node>, message: String, category: Category| {
        let (file, range) = node_range(node);
        let d = new_diagnostic(file, range, new_ad_hoc_message(&message), &[]);
        if category != Category::Error {
            d.set_category(category);
        }
        d
    };
    let message = match &diagnostic.hint {
        Some(hint) => format!("{}\n{hint}", diagnostic.message),
        None => diagnostic.message.clone(),
    };
    let d = at(diagnostic.node, message, diagnostic.category);
    if !diagnostic.related.is_empty() {
        let related: Vec<P<Diagnostic>> =
            diagnostic.related.iter().map(|(node, message)| at(*node, message.clone(), Category::Message)).collect();
        d.set_related_info(&related);
    }
    d
}

impl HookBackend<'_, '_> {
    fn name(&self) -> &'static str {
        self.host.name(self.plugin)
    }

    /// Something the running hook did that its node can no longer be declined after (on its own frame).
    fn note_effect(&mut self, what: &'static str) {
        note_effect(self.state, what);
    }

    /// A temporary identifier no user local of the file can capture ([`PluginFileState::seam_hint`]).
    fn seam_temp(&mut self, hint: &str) -> LuauIdent {
        temp_id(self.state.plugin_file.seam_hint(self.state.source_file, hint))
    }

    fn hoist_unchecked(&mut self, value: Expr, hint: &str) -> Expr {
        self.note_effect("calling hoist");
        let id = self.seam_temp(hint);
        self.state.plugin_file.main_chunk_locals += 1;
        self.state.plugin_file.root_inserts.push(create::local(id.clone(), Some(value)));
        Expr::TemporaryIdentifier(id)
    }

    /// Records the internal error for a call the phase table refuses, and says whether it was refused.
    fn refused(&mut self, what: &str, row: Row) -> bool {
        let allowed = match (self.phase, row) {
            (Phase::Claim, _) => true,
            (Phase::File, Row::Lowering) => false,
            (Phase::File, _) => true,
            (Phase::Classify | Phase::Explain, _) => false,
        };
        if !allowed {
            let message = format!("plugin `{}` called {what} in {}", self.name(), self.hook.label());
            self.state.internal_error(self.anchor, &message);
        }
        !allowed
    }

    /// An internal error at the anchor, naming the plugin.
    fn misuse(&mut self, message: &str) -> Aborted {
        let message = format!("plugin `{}`: {message}", self.name());
        self.state.internal_error(self.anchor, &message);
        Aborted
    }

    /// The prereq scope the lowering primitives write to.
    fn scope(&mut self) -> &mut Prereqs {
        match self.captures.last_mut() {
            Some(capture) => capture,
            None => self.prereqs.as_deref_mut().expect("a claim hook has a prereq scope"),
        }
    }

    /// Runs compiler code for the plugin: refused outside claim hooks, timed as the compiler's, and `Err` when it
    /// reported an error.
    fn lower<T>(
        &mut self,
        what: &str,
        f: impl FnOnce(&mut TransformState<'_>, &mut Prereqs, Option<&mut CallContext>) -> Result<T, String>,
    ) -> Result<T, Aborted> {
        if self.refused(what, Row::Lowering) {
            return Err(Aborted);
        }
        self.lower_unchecked(f)
    }

    /// [`HookBackend::lower`] after the phase check.
    fn lower_unchecked<T>(
        &mut self,
        f: impl FnOnce(&mut TransformState<'_>, &mut Prereqs, Option<&mut CallContext>) -> Result<T, String>,
    ) -> Result<T, Aborted> {
        let before = self.state.diagnostics.diagnostics.len();
        self.host.enter(None, self.hook);
        let prereqs = match self.captures.last_mut() {
            Some(capture) => capture,
            None => self.prereqs.as_deref_mut().expect("a claim hook has a prereq scope"),
        };
        let result = f(self.state, prereqs, self.call.as_deref_mut());
        self.host.exit();
        match result {
            Ok(value) if !has_errors_since(self.state, before) => Ok(value),
            Ok(_) => Err(Aborted),
            Err(message) => Err(self.misuse(&message)),
        }
    }

    /// The call context for a `CallSite` the plugin passed: it must be the call being claimed.
    fn check_site(&mut self, call: &CallSite, what: &str) -> Result<(), Aborted> {
        match self.call.as_deref() {
            Some(context) if context.site.node == call.node => Ok(()),
            _ => Err(self.misuse(&format!("{what} needs the CallSite of the call being claimed"))),
        }
    }
}

impl CxBackend for HookBackend<'_, '_> {
    fn program(&self) -> &'static Program {
        self.state.program
    }

    fn options(&self) -> P<CompilerOptions> {
        self.state.program.options()
    }

    fn file(&self) -> &FileInfo {
        self.state.plugin_file.info.as_ref().expect("a file being transformed for plugins")
    }

    fn checker(&mut self) -> &mut Checker {
        &mut *self.state.checker
    }

    fn project(&self) -> &ProjectServices {
        self.state.services
    }

    fn resolve_module(&self, specifier: &str, from: P<SourceFile>) -> Option<ResolvedModuleInfo> {
        self.host.resolve_module(specifier, from.get().file_name())
    }

    fn declaring_package(&self, symbol: P<Symbol>) -> Option<Arc<PackageInfo>> {
        PluginHost::declaring_package(self.state.services, symbol)
    }

    // --- lowering ---

    fn compile_expr(&mut self, node: P<Node>) -> Result<Expr, Aborted> {
        if self.call.as_deref().is_some_and(|context| context.site.node == node) {
            return self.lower("compile_expr", |state, prereqs, context| {
                compile_claimed_call(state, prereqs, context.expect("a call context"))
            });
        }
        if self.hook == HookKind::Expression && node == self.anchor {
            self.state.plugin_file.offered_expressions.insert(node);
            self.note_effect("lowering it with compile_expr");
        }
        self.lower("compile_expr", |state, prereqs, _| Ok(transform_expression(state, prereqs, node)))
    }

    fn compile_in_order(&mut self, nodes: &[P<Node>]) -> Result<Vec<Expr>, Aborted> {
        self.lower("compile_in_order", |state, prereqs, _| Ok(ensure_transform_order(state, prereqs, nodes)))
    }

    fn compile_receiver(&mut self, call: &CallSite) -> Result<Expr, Aborted> {
        if self.refused("compile_receiver", Row::Lowering) {
            return Err(Aborted);
        }
        self.check_site(call, "compile_receiver")?;
        let in_capture = !self.captures.is_empty();
        self.lower_unchecked(|state, prereqs, context| {
            let context = context.expect("checked above");
            compile_receiver(state, prereqs, context, in_capture)
        })
    }

    fn compile_call(&mut self, call: &CallSite, callee: Callee, args: Vec<Arg>) -> Result<Expr, Aborted> {
        if self.refused("compile_call", Row::Lowering) {
            return Err(Aborted);
        }
        self.check_site(call, "compile_call")?;
        self.lower_unchecked(|state, prereqs, context| {
            let context = context.expect("checked above");
            if let Some(frame) = state.plugin_file.frames.last_mut() {
                frame.called = true;
            }
            note_effect(state, "lowering it with compile_call");
            lower_call(state, prereqs, context, callee, args)
        })
    }

    fn compile_object_literal(&mut self, node: P<Node>, _edits: ObjectEdits) -> Result<Expr, Aborted> {
        if self.refused("compile_object_literal", Row::Lowering) {
            return Err(Aborted);
        }
        // rbxts-rs: S2 — object literal edits (rename, replace, append, reorder) on roblox-ts's object lowering
        let _ = node;
        Err(self.misuse("compile_object_literal is not supported yet in S1"))
    }

    fn compile_binary(&mut self, _op: BinOp, _left: P<Node>, _right: P<Node>) -> Result<Expr, Aborted> {
        if self.refused("compile_binary", Row::Lowering) {
            return Err(Aborted);
        }
        // rbxts-rs: S2 — binary operators on original operands, with the assignment hook
        Err(self.misuse("compile_binary is not supported yet in S1"))
    }

    // --- prereqs and placement ---

    fn prereq(&mut self, stmt: Stmt) {
        if !self.refused("prereq", Row::Lowering) {
            self.scope().push(stmt);
        }
    }

    fn push_to_var(&mut self, value: Expr, hint: &str) -> Expr {
        if self.refused("push_to_var", Row::Lowering) {
            return create::nil();
        }
        // an empty hint is the one roblox-ts derives from the value, made capture-safe like any other
        let hint = if hint.is_empty() { value_to_id_str(&value) } else { hint.to_owned() };
        let hint = self.state.plugin_file.seam_hint(self.state.source_file, &hint);
        self.scope().push_to_var(value, &hint)
    }

    fn begin_capture(&mut self) {
        // a refused capture still opens a scope, so begin and end stay paired
        self.refused("capture", Row::Lowering);
        self.captures.push(Prereqs::new());
    }

    fn end_capture(&mut self) -> Vec<Stmt> {
        self.captures.pop().expect("end_capture after begin_capture").statements
    }

    fn insert_before_root_statement(&mut self, stmts: Vec<Stmt>) {
        if !self.refused("insert_before_root_statement", Row::Lowering) {
            self.note_effect("calling insert_before_root_statement");
            self.state.plugin_file.main_chunk_locals += stmts.iter().map(declared_locals).sum::<u32>();
            self.state.plugin_file.root_inserts.extend(stmts);
        }
    }

    fn header(&mut self, stmts: Vec<Stmt>) {
        if !self.refused("header", Row::FileLevel) {
            self.note_effect("calling header");
            self.state.plugin_file.main_chunk_locals += stmts.iter().map(declared_locals).sum::<u32>();
            self.state.plugin_file.header.extend(stmts);
        }
    }

    fn hoist(&mut self, value: Expr, hint: &str) -> Expr {
        if self.refused("hoist", Row::Lowering) {
            return create::nil();
        }
        self.hoist_unchecked(value, hint)
    }

    fn hoist_spillable(&mut self, value: Expr, hint: &str) -> Expr {
        if self.refused("hoist_spillable", Row::Lowering) {
            return create::nil();
        }
        // rbxts-rs: TODO S2 — the spill table: when the main chunk would pass Luau's 200-local limit, spillable
        // hoists become fields of one table local, and the compiler reports the limit only after spilling.
        self.hoist_unchecked(value, hint)
    }

    fn import_named(&mut self, specifier: &str, export: &str) -> Result<Expr, Aborted> {
        if self.refused("import_named", Row::FileLevel) {
            return Err(Aborted);
        }
        self.note_effect("calling import_named");
        let anchor = self.anchor;
        let before = self.state.diagnostics.diagnostics.len();
        match dispatch::import_named(self.state, self.host, anchor, specifier, export) {
            Ok(value) if !has_errors_since(self.state, before) => Ok(value),
            Ok(_) => Err(Aborted),
            Err(message) => Err(self.misuse(&message)),
        }
    }

    fn mark_import_used(&mut self, alias_declaration: P<Node>) {
        if !self.refused("mark_import_used", Row::FileLevel) {
            self.state.plugin_file.forced_imports.insert(alias_declaration);
        }
    }

    fn request_import_elision(&mut self, alias_declaration: P<Node>) {
        if self.refused("request_import_elision", Row::FileLevel) {
            return;
        }
        if !matches!(
            alias_declaration.kind(),
            Kind::ImportSpecifier | Kind::ImportClause | Kind::NamespaceImport | Kind::ImportEqualsDeclaration
        ) {
            self.misuse(&format!(
                "request_import_elision takes an import alias declaration, not a {:?}",
                alias_declaration.kind()
            ));
            return;
        }
        let requests = &mut self.state.plugin_file.elision_requests;
        if !requests.contains(&alias_declaration) {
            requests.push(alias_declaration);
        }
    }

    fn temp_id(&mut self, hint: &str) -> LuauIdent {
        self.refused("temp_id", Row::FileLevel);
        self.seam_temp(hint)
    }

    /// A valid identifier, not a name roblox-ts's output uses itself (`luau.globals`), not a name a plugin import
    /// took, and not a name the file's source uses anywhere, which covers every global (Roblox's or declared) the
    /// output refers to and every local.
    fn is_usable_local_name(&self, name: &str) -> bool {
        self.state.plugin_file.approve_name(self.state.source_file, name)
    }

    fn locals(&self) -> LocalCounts {
        let main_chunk = self.state.plugin_file.main_chunk_locals;
        // rbxts-rs: TODO S2 — locals of the function being built are not counted yet; at the main-chunk level the
        // two counts are the same.
        let in_function = crate::util::traversal::get_ancestor(self.anchor, tsrs_ast::is_function_like).is_some();
        LocalCounts { function: if in_function { 0 } else { main_chunk }, main_chunk }
    }

    fn late_string(&mut self, key: &str) -> Expr {
        if self.refused("late_string", Row::FileLevel) {
            return create::nil();
        }
        let key = super::lifecycle::late_key(self.name(), key);
        self.state.plugin_file.use_late_key(self.plugin, key.clone());
        create::late_string(key)
    }

    fn comment(&mut self, text: &str) {
        if !self.refused("comment", Row::Lowering) {
            self.scope().push(create::comment(text));
        }
    }

    // --- source facts ---

    fn utf16_pos(&self, node: P<Node>) -> Utf16Pos {
        let Some(file) = tsrs_ast::get_source_file_of_node(node) else { return Utf16Pos::default() };
        let Some((start, _)) = facts::span(node, file) else { return Utf16Pos::default() };
        facts::utf16_pos_of(file, &self.host.lines(file), start)
    }

    fn utf16_width(&self, node: P<Node>) -> u32 {
        let Some(file) = tsrs_ast::get_source_file_of_node(node) else { return 0 };
        let Some((start, end)) = facts::span(node, file) else { return 0 };
        facts::utf16_len(&self.host.lines(file), start, end)
    }

    fn source_text(&self, node: P<Node>) -> &'static str {
        let Some(file) = tsrs_ast::get_source_file_of_node(node) else { return "" };
        let Some((start, end)) = facts::span(node, file) else { return "" };
        &file.get().text()[start as usize..end as usize]
    }

    fn jsdoc_tags(&mut self, declaration: P<Node>) -> Rc<[JsDocTag]> {
        self.host.jsdoc_tags(self.state.checker, declaration)
    }

    fn file_diagnostics(&self) -> &[P<Diagnostic>] {
        let file = self.file().source;
        self.host.file_diagnostics.get(&file).map_or(&[], Vec::as_slice)
    }

    // --- diagnostics ---

    fn report(&mut self, diagnostic: PluginDiagnostic) {
        if self.phase == Phase::Classify {
            let message =
                format!("plugin `{}` reported a diagnostic from classify_symbol: {}", self.name(), diagnostic.message);
            self.state.internal_error(diagnostic.node, &message);
            return;
        }
        self.state.add_diagnostic(to_diagnostic(&diagnostic));
    }
}

// --- compile_receiver and compile_call ---

/// The receiver node of a property or element call.
fn receiver_node(shape: CallShape) -> Option<P<Node>> {
    match shape {
        CallShape::Property { access, .. } => Some(access.as_property_access_expression().expression),
        CallShape::Element { access, .. } => Some(access.as_element_access_expression().expression),
        _ => None,
    }
}

/// Records on the running hook's frame something its node can no longer be declined after.
fn note_effect(state: &mut TransformState<'_>, what: &'static str) {
    if let Some(frame) = state.plugin_file.frames.last_mut() {
        frame.lowered.get_or_insert(what);
    }
}

/// `compile_receiver`: the receiver as the chain lowering compiled it, or compiled now (once). Compiled into the
/// offer's own scope, it is kept for the next plugin (or roblox-ts) to reuse if this one declines; compiled inside a
/// capture, its prereqs are the plugin's to place, so the call cannot be declined any more.
fn compile_receiver(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    context: &mut CallContext,
    in_capture: bool,
) -> Result<Expr, String> {
    let Some(receiver) = receiver_node(context.shape) else {
        return Err(format!("compile_receiver: a {:?} call has no receiver", context.site.kind));
    };
    if let Some(base) = &context.base {
        return Ok(base.clone());
    }
    let before = state.plugin_file.effect_mark(prereqs.statements.len());
    let nested_before = state.plugin_file.frames.last().map_or(0, |f| f.nested_hooks);
    let value = transform_expression(state, prereqs, receiver);
    context.base = Some(value.clone());
    if in_capture {
        context.base_captured = true;
        note_effect(state, "compiling the receiver inside a capture");
    } else {
        context.base_marks = Some((before, state.plugin_file.effect_mark(prereqs.statements.len())));
        if let Some(frame) = state.plugin_file.frames.last_mut() {
            frame.receiver_nested_hooks += frame.nested_hooks - nested_before;
        }
    }
    Ok(value)
}

/// `compile_expr` of the claimed call itself: the call lowered as roblox-ts lowers it, without asking the plugins
/// again. A chain call whose base the chain lowering (or `compile_receiver`) already compiled is lowered on that
/// base, so the receiver is not evaluated twice; a call nothing of which is compiled yet is lowered from scratch.
fn compile_claimed_call(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    context: &mut CallContext,
) -> Result<Expr, String> {
    if state.plugin_file.frames.last().is_some_and(|f| f.called) {
        return Err("compile_expr of the claimed call after compile_call lowered it".to_owned());
    }
    note_effect(state, "lowering it with compile_expr");
    let node = context.site.node;
    if let CallShape::Decorator { call } = context.shape {
        // the decorator's expression, its call (offered as this decorator) not asked about again
        if let Some(call) = call {
            state.plugin_file.offered.insert(call);
        }
        let expression = node.expression().expect("a decorator has an expression");
        return Ok(transform_expression(state, prereqs, expression));
    }
    // the call is not offered again on the way down
    state.plugin_file.offered.insert(node);
    match (&context.base, context.item) {
        (Some(base), Some(item)) => Ok(transform_chain_item(state, prereqs, base.clone(), &item)),
        (Some(_), None) => Err("compile_expr of the claimed call after its callee was compiled".to_owned()),
        (None, _) => Ok(transform_expression(state, prereqs, node)),
    }
}

/// The original callee's base, compiled now unless the chain lowering or `compile_receiver` already did.
fn ensure_base(state: &mut TransformState<'_>, prereqs: &mut Prereqs, context: &mut CallContext) -> Expr {
    if let Some(base) = &context.base {
        return base.clone();
    }
    let value = match context.shape {
        CallShape::Call => transform_expression(state, prereqs, context.site.callee),
        CallShape::Super => transform_super_keyword(),
        CallShape::Property { .. } | CallShape::Element { .. } => {
            transform_expression(state, prereqs, receiver_node(context.shape).expect("a receiver"))
        }
        CallShape::New => {
            validate_not_any_type(state, context.site.callee);
            convert_to_indexable_expression(transform_expression(state, prereqs, context.site.callee))
        }
        CallShape::Decorator { .. } => transform_expression(state, prereqs, context.site.callee),
    };
    context.base = Some(value.clone());
    value
}

/// A roblox-ts macro that would lower this call: `compile_call` with the original callee cannot pass edited
/// arguments to it.
fn original_macro_name(state: &mut TransformState<'_>, context: &CallContext) -> Option<&'static str> {
    match context.shape {
        CallShape::New => {
            let symbol = get_first_construct_symbol(state, context.site.callee)?;
            state.macro_manager.get_constructor_macro(symbol).map(|m| m.name)
        }
        CallShape::Call | CallShape::Property { .. } | CallShape::Element { .. } => {
            let callee = context.site.callee;
            let callee_type = state.get_type(callee);
            let exp_type = state.checker.get_non_optional_type(callee_type);
            let symbol = get_first_defined_symbol(state, exp_type)?;
            match context.shape {
                CallShape::Call => state.macro_manager.get_call_macro(state.checker, symbol).map(|m| m.name),
                _ => state.macro_manager.get_property_call_macro(state.checker, symbol).map(|m| m.name),
            }
        }
        CallShape::Super | CallShape::Decorator { .. } => None,
    }
}

/// The argument slots, compiled in evaluation order: each `Original` node with its own prereqs, then the values
/// bound to locals where a later slot's prereqs could change them (`ensureTransformOrder` over every slot, ready
/// Luau values included). Returns the values, the slot nodes (`None` for Luau and `nil` slots) and the prereqs.
fn compile_slots(state: &mut TransformState<'_>, args: Vec<Arg>) -> (Vec<Expr>, Vec<Option<P<Node>>>, Prereqs) {
    let mut infos: Vec<(Expr, Vec<Stmt>, Option<P<Node>>)> = Vec::with_capacity(args.len());
    for arg in args {
        match arg {
            Arg::Original(node) => {
                let mut node_prereqs = Prereqs::new();
                let value = transform_expression(state, &mut node_prereqs, node);
                infos.push((value, node_prereqs.statements, Some(node)));
            }
            Arg::Luau(value) => infos.push((value, Vec::new(), None)),
            Arg::Nil => infos.push((create::nil(), Vec::new(), None)),
        }
    }
    let mut captures = vec![false; infos.len()];
    let mut suffix = Effects::none();
    for i in (0..infos.len()).rev() {
        let (expression, statements, _) = &infos[i];
        let effects = get_effects(expression);
        captures[i] = !effects_commute(&effects, &suffix);
        if captures[i] {
            suffix = join_effects(effects, &suffix);
        }
        suffix = join_effects(get_effects_of_statements(statements), &suffix);
    }
    let mut prereqs = Prereqs::new();
    let mut values = Vec::with_capacity(infos.len());
    let mut nodes = Vec::with_capacity(infos.len());
    for ((expression, statements, node), capture) in infos.into_iter().zip(captures) {
        prereqs.push_list(statements);
        values.push(if capture { prereqs.push_to_var(expression, "exp") } else { expression });
        nodes.push(node);
    }
    (values, nodes, prereqs)
}

/// `fixVoidArgumentsForRobloxFunctions` on the `Original` slots.
fn fix_void_slots(
    state: &mut TransformState<'_>,
    context: &CallContext,
    values: &mut [Expr],
    nodes: &[Option<P<Node>>],
) {
    let callee_type = state.get_type(context.site.callee);
    let exp_type = state.checker.get_non_optional_type(callee_type);
    for (value, node) in values.iter_mut().zip(nodes) {
        if let Some(node) = node {
            fix_void_arguments_for_roblox_functions(state, exp_type, std::slice::from_mut(value), &[*node]);
        }
    }
}

/// `compile_call`: the call with another callee and/or argument slots, lowered as roblox-ts lowers calls.
fn lower_call(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    context: &mut CallContext,
    callee: Callee,
    args: Vec<Arg>,
) -> Result<Expr, String> {
    let node = context.site.node;
    match callee {
        Callee::Original => {
            if let CallShape::Decorator { call: None } = context.shape {
                return Err("compile_call: `@decorator` without a call has no call to lower".to_owned());
            }
            if let Some(name) = original_macro_name(state, context) {
                return Err(format!(
                    "compile_call: `{name}` is a roblox-ts macro; Callee::Original cannot lower it with argument \
                     slots (lower the node with compile_expr, or call another function)"
                ));
            }
            if matches!(context.shape, CallShape::Call | CallShape::Property { .. } | CallShape::Element { .. }) {
                validate_not_any_type(state, context.site.callee);
            }
            let mut base = ensure_base(state, prereqs, context);
            match context.shape {
                CallShape::Super => {
                    let (values, _, arg_prereqs) = compile_slots(state, args);
                    prereqs.push_list(arg_prereqs.statements);
                    let mut with_self = vec![rbxts_luau::globals::self_()];
                    with_self.extend(values);
                    Ok(create::call(create::property(convert_to_indexable_expression(base), "constructor"), with_self))
                }
                CallShape::New => {
                    let (values, _, arg_prereqs) = compile_slots(state, args);
                    prereqs.push_list(arg_prereqs.statements);
                    Ok(create::call(create::property(base, "new"), values))
                }
                CallShape::Call | CallShape::Decorator { .. } => {
                    let (mut values, nodes, arg_prereqs) = compile_slots(state, args);
                    let callee_node = context.site.callee;
                    fix_void_slots(state, context, &mut values, &nodes);
                    if !effects_commute(&get_effects(&base), &get_effects_of_statements(&arg_prereqs.statements)) {
                        base = prereqs.push_to_var(base, "fn");
                    }
                    prereqs.push_list(arg_prereqs.statements);
                    let callee_type = state.get_type(callee_node);
                    let exp_type = state.checker.get_non_optional_type(callee_type);
                    if is_method_from_type(state, callee_node, exp_type) {
                        values.insert(0, rbxts_luau::Expr::Nil);
                    }
                    let call = create::call(convert_to_indexable_expression(base), values);
                    Ok(match context.shape {
                        CallShape::Call => wrap_return_if_lua_tuple(state, node, call),
                        _ => call,
                    })
                }
                CallShape::Property { access, name } => {
                    let (mut values, nodes, arg_prereqs) = compile_slots(state, args);
                    fix_void_slots(state, context, &mut values, &nodes);
                    let method = is_method(state, access);
                    let call = create_ordered_property_call(
                        state,
                        prereqs,
                        access,
                        base,
                        PropertyKey::Name(name.to_owned()),
                        values,
                        arg_prereqs.statements,
                        method,
                    );
                    Ok(wrap_return_if_lua_tuple(state, node, call))
                }
                CallShape::Element { access, argument } => {
                    let mut key_prereqs = Prereqs::new();
                    let argument_value = transform_expression(state, &mut key_prereqs, argument);
                    let later = get_effects_of_statements(&key_prereqs.statements);
                    if !effects_commute(&get_effects(&base), &later) {
                        base = prereqs.push_to_var(base, "exp");
                    }
                    prereqs.push_list(key_prereqs.statements);
                    let (mut values, nodes, arg_prereqs) = compile_slots(state, args);
                    fix_void_slots(state, context, &mut values, &nodes);
                    let receiver_type = state.get_type(access.as_element_access_expression().expression);
                    let receiver_type = state.checker.get_non_optional_type(receiver_type);
                    let key = add_one_if_array_type(state, receiver_type, argument_value);
                    let method = is_method(state, access);
                    let call = create_ordered_property_call(
                        state,
                        prereqs,
                        access,
                        base,
                        PropertyKey::Index(key),
                        values,
                        arg_prereqs.statements,
                        method,
                    );
                    Ok(wrap_return_if_lua_tuple(state, node, call))
                }
            }
        }
        Callee::Function(function) => {
            let (values, _, arg_prereqs) = compile_slots(state, args);
            let function =
                if effects_commute(&get_effects(&function), &get_effects_of_statements(&arg_prereqs.statements)) {
                    function
                } else {
                    prereqs.push_to_var(function, "fn")
                };
            prereqs.push_list(arg_prereqs.statements);
            Ok(create::call(convert_to_indexable_expression(function), values))
        }
        Callee::Method { object, name } => {
            if !rbxts_luau::is_valid_identifier(&name) {
                return Err(format!(
                    "compile_call: Callee::Method name `{name}` is not a valid Luau identifier (use Callee::Function \
                     with the object as the first argument)"
                ));
            }
            let (values, _, arg_prereqs) = compile_slots(state, args);
            let object = if effects_commute(&get_effects(&object), &get_effects_of_statements(&arg_prereqs.statements))
            {
                object
            } else {
                prereqs.push_to_var(object, "self")
            };
            prereqs.push_list(arg_prereqs.statements);
            Ok(create::method_call(convert_to_indexable_expression(object), name, values))
        }
    }
}

/// The `CallKind` of a property or element call: a method call when roblox-ts calls it with `:`.
pub(crate) fn access_call_kind(state: &mut TransformState<'_>, access: P<Node>) -> CallKind {
    if is_method(state, access) { CallKind::Method } else { CallKind::Call }
}
