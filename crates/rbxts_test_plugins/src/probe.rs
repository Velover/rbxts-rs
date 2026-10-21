//! `probe`: a test plugin for the compiler's side of the seam. Every primitive the compiler offers is reached from one
//! fixture (`crates/rbxts_transform/tests/plugin_fixtures/probe`).
//!
//! **Calls by symbol**: a function whose JSDoc has `@probe <op>`; `classify_symbol` reads the op:
//! - `hoist`: `cx.hoist(arg)`, a local before the root statement;
//! - `prereq`: the argument bound with `push_to_var`, after a `comment`;
//! - `order`: `compile_in_order(args)` as an array;
//! - `inject`: `compile_call(Original, [Original(arg0), Luau("injected"), Nil])`;
//! - `swap`: `compile_call(Function(import_named("./runtime", "impl")), [Original(arg1), Original(arg0)])`;
//! - `late`: a late string, resolved in `finish` to `"<plugin>#<n>"` numbered over all files in program order;
//! - `capture`: `capture(compile_expr(arg))`, wrapped as `(function() <prereqs> return v end)()`;
//! - `root`: `insert_before_root_statement([comment])` and `header([comment])`, value `nil`;
//! - `elide`: the call becomes its callee's name, and its import alias is handed to `request_import_elision`;
//! - `keep`: like `elide`, and `mark_import_used` keeps the import anyway;
//! - `new` (on a class): `new C(args)` → `compile_call(Original, [Luau("probed"), Original(args)…])`;
//! - `decorator`: `@d()` → `compile_call(Original, [Luau("probed")])`;
//! - `self`: `wrap(compile_expr(call.node))`: the claimed call lowered as roblox-ts lowers it;
//! - `declineafter` / `hoistdecline` / `prereqdecline` / `argdecline`: declined after `compile_expr` of the
//!   call, a hoist, a prereq, or `compile_expr` of an argument whose own claim hoisted (each an internal error);
//! - `decoself`: a decorator lowered with `compile_expr(call.node)`;
//! - `approve` / `approveimpl`: declares a local named `_hoisted` / `impl` when `is_usable_local_name` approves
//!   it, then compiles the argument (seam temporaries and imports must avoid approved names, in either order);
//! - `caprecv`: `capture(compile_receiver)`, statements dropped, then declined (an internal error);
//! - `inorderself`: `wrap(compile_in_order([call.node]))`, the claimed node through `compile_in_order`;
//! - `latedecline` / `markdecline`: a late string, or `mark_import_used` of the argument's import, then declined
//!   (each an internal error);
//! - `twice`: `compile_expr(call.node)` twice (the second is an internal error);
//! - `recvarg`: `compile_call(Function(useRecv), [Luau(compile_receiver), Original(args)…])` (on optional and
//!   non-method calls the receiver is bound to a local); `recvsuper`: `compile_receiver` of `super.m()` (an
//!   internal error);
//! - `objedit`: `compile_object_literal` renaming `a`, replacing `b` and appending `e`; `objorder`: appending `w` and
//!   emitting the fields in another order;
//! - `binary`: `compile_binary(PlusToken, a, b)`; `locals`: the `locals()` counts as a string; `spill`:
//!   `hoist_spillable(compile_expr(a))`;
//! - `calltwice` / `exprthencall` / `ancestor` / `binbad`: `compile_call` twice, `compile_expr` then `compile_call`,
//!   `compile_expr` of the call's parent, `compile_binary` with `+=` (each an internal error); `recvexpr`:
//!   `compile_expr(call.receiver)`, an internal error where the receiver is compiled already;
//! - `facts`: a string of source facts: the call's UTF-16 line and character, its width, the callee's declaring
//!   package, where `./runtime` resolves, the `{@link}` in the tag and what it resolves to, the file's checker
//!   diagnostics;
//! - `boom`: panics in [`Mode::Panic`]; `fail`: an error in [`Mode::Fail`]; `err`: `Err` with no new error in
//!   [`Mode::ErrWithoutError`]; declined otherwise.
//!
//! **By receiver**: `obj.probeMethod(x)` where `obj`'s type has `__probeMarker` →
//! `compile_call(Method { compile_receiver, "probed" }, [Original(x)])`.
//! **By callee marker**: `f(x)` where `f`'s type has `__probeCallee` → `compile_call(Original, [Original(x),
//! Luau(1)])`.
//! **Property keys**: accesses whose receiver has `__probeKey` get the key `k_<key>` (`key with space` for `swap`, the
//! written key for `same`).
//! **Assignments**: to `this.<p>.key` where `p`'s JSDoc has `@probeattrs`: `self:setAttribute("key", value[, true])`
//! (compound values with `compile_binary`); `@probeattrsread` compiles the target whatever the operation.
//! **Class hook** (decorated classes): `Class.__probed = "<name>"`; [`Mode::ClassSelf`] compiles a class expression
//! itself (an internal error).
//! **By kind**: `void probeVoid` becomes `"void claimed"` (other `void`s are declined); `debugger` becomes
//! `print("probe: debugger")`.
//! **explain_checker_diagnostic**: a checker message naming `probeBroken` is replaced.
//! **Finish**: an artifact `probe-report.json` with the claims per file; in [`Mode::Unresolved`] one late key is left
//! unresolved, in [`Mode::ArtifactOutside`] an artifact goes outside the project.
//! **[`Mode::Refuse`]** calls refused primitives in each phase: `prereq` in `begin_file`, `late_string` in
//! `classify_symbol`, `hoist` in `end_file`.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use rbxts_luau::{AssignmentOperator, BinaryOperator, create};
use rbxts_plugin::ts::{Diagnostic, Kind, Node, P, Symbol};
use rbxts_plugin::{
    Abort, AccessSite, Arg, AssignOp, AssignSite, CallKind, CallSite, Callee, CheckerInfo, Claim, ClassFilter,
    ClassSite, Cx, Fatal, FieldOrder, FileIndex, FinishCx, Interest, LuauExpr, LuauStmt, ObjectEdits, PResult, Plugin,
    PluginDiagnostic, PluginInfo, PluginInstance, SetupCx, SymbolRole, Tag, text_literal,
};

/// How a probe misbehaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// `@probe boom` panics.
    Panic,
    /// Refused primitives in `begin_file`, `classify_symbol` and `end_file`.
    Refuse,
    /// `finish` leaves the last late key unresolved.
    Unresolved,
    /// `@probe fail` reports an error.
    Fail,
    /// `@probe err` returns `Err` without a new error (the first such call reports one and keeps its `Abort`).
    ErrWithoutError,
    /// `finish` writes an artifact outside the project.
    ArtifactOutside,
    /// `finish` writes an artifact through a `link` junction and one with a trailing `.` on a Luau output.
    ArtifactSneaky,
    /// `setup` panics.
    SetupPanic,
    /// `finish` panics.
    FinishPanic,
    /// The class hook for every class, compiling a class expression itself (an internal error).
    ClassSelf,
    /// Only calls by symbol; every call hook takes a temporary id, then declines (for `RBXTS_PLUGIN_CLAIMS=all`).
    DeclineTouched,
    /// Only an expression-kind interest in calls and `new` (`probeExpr…` / `ProbeExpr…` callees): the hook wraps
    /// `compile_expr` of its own node, so the call plugins still claim the call inside; and a statement hook on
    /// `probeStmtSelf;` that compiles its own statement (an internal error).
    ExprCall,
}

/// The probe factory. Two probes with different names can be registered together (claim order tests).
pub struct Probe {
    pub name: &'static str,
    pub mode: Mode,
}

impl Probe {
    pub fn new(name: &'static str, mode: Mode) -> Probe {
        Probe { name, mode }
    }
}

const OPS: &[&str] = &[
    "hoist",
    "prereq",
    "order",
    "inject",
    "swap",
    "late",
    "capture",
    "root",
    "elide",
    "keep",
    "new",
    "decorator",
    "boom",
    "fail",
    "err",
    "facts",
    "self",
    "declineafter",
    "hoistdecline",
    "prereqdecline",
    "argdecline",
    "decoself",
    "approve",
    "approveimpl",
    "caprecv",
    "inorderself",
    "latedecline",
    "markdecline",
    "twice",
    "recvarg",
    "recvsuper",
    "objedit",
    "objorder",
    "binary",
    "locals",
    "spill",
    "calltwice",
    "exprthencall",
    "ancestor",
    "binbad",
    "recvexpr",
];
const RECEIVER: Tag = Tag(100);
const CALLEE: Tag = Tag(101);
const VOID: Tag = Tag(102);
const DEBUGGER: Tag = Tag(103);
const EXPRCALL: Tag = Tag(105);
const STMTSELF: Tag = Tag(106);
const KEY: Tag = Tag(107);
const ATTRS: Tag = Tag(108);
const ATTRSREAD: Tag = Tag(109);
const EXPRBIN: Tag = Tag(110);

fn op(tag: Tag) -> &'static str {
    OPS.get(tag.0 as usize).copied().unwrap_or("?")
}

pub struct ProbeShared {
    name: &'static str,
    mode: Mode,
}

/// One file's claims and late keys.
#[derive(Debug, Default)]
pub struct ProbePart {
    pub file: String,
    pub claims: u32,
    pub late_keys: Vec<String>,
}

pub struct ProbeInstance {
    name: &'static str,
    mode: Mode,
    claims: Cell<u32>,
    late_keys: RefCell<Vec<String>>,
    header_done: Cell<bool>,
    saved: RefCell<Option<Abort>>,
}

impl Plugin for Probe {
    type Shared = ProbeShared;
    type State = ();
    type FilePart = ProbePart;
    type Instance = ProbeInstance;

    fn info(&self) -> PluginInfo {
        PluginInfo { name: self.name, version: "0.1.0", tsconfig_transform: None }
    }

    fn setup(&self, cx: &mut SetupCx<'_>, _: &mut ()) -> Result<Arc<ProbeShared>, Fatal> {
        if self.mode == Mode::SetupPanic {
            panic!("probe: setup panic");
        }
        cx.info("probe set up");
        Ok(Arc::new(ProbeShared { name: self.name, mode: self.mode }))
    }

    fn instance(&self, shared: Arc<ProbeShared>, _: CheckerInfo) -> ProbeInstance {
        ProbeInstance {
            name: shared.name,
            mode: shared.mode,
            claims: Cell::new(0),
            late_keys: RefCell::new(Vec::new()),
            header_done: Cell::new(false),
            saved: RefCell::new(None),
        }
    }

    fn finish(
        &self,
        shared: &ProbeShared,
        _: &mut (),
        parts: &[(FileIndex, &ProbePart)],
        cx: &mut FinishCx<'_>,
    ) -> Result<(), Fatal> {
        let keys: Vec<&String> = parts.iter().flat_map(|(_, part)| &part.late_keys).collect();
        let resolve = match shared.mode {
            Mode::Unresolved => keys.len().saturating_sub(1),
            _ => keys.len(),
        };
        for (n, key) in keys.iter().take(resolve).enumerate() {
            cx.resolve_late(key, &format!("{}#{}", shared.name, n + 1));
        }
        let files: Vec<String> = parts
            .iter()
            .filter(|(_, part)| part.claims > 0)
            .map(|(_, part)| format!("{{\"file\":\"{}\",\"claims\":{}}}", part.file, part.claims))
            .collect();
        cx.write_artifact(format!("{}-report.json", shared.name), format!("[{}]\n", files.join(",")).into_bytes());
        match shared.mode {
            Mode::ArtifactOutside => cx.write_artifact("../outside-the-project.txt", b"no".to_vec()),
            Mode::ArtifactSneaky => {
                cx.write_artifact("out/shared/calls.luau.", b"-- not a second spelling of calls.luau\n".to_vec());
                cx.write_artifact("link/through-the-junction.txt", b"no".to_vec());
            }
            Mode::FinishPanic => panic!("probe: finish panic"),
            _ => {}
        }
        Ok(())
    }
}

impl ProbeInstance {
    fn claimed(&self, value: LuauExpr) -> PResult<Claim<LuauExpr>> {
        self.claims.set(self.claims.get() + 1);
        Ok(Claim::Take(value))
    }

    /// The import alias declaration a callee identifier refers to, if it is an import.
    fn alias_of(cx: &mut Cx<'_>, callee: P<Node>) -> Option<P<Node>> {
        let symbol = cx.checker().get_symbol_at_location_exported(callee)?;
        symbol.declarations().iter().copied().find(|d| {
            matches!(
                d.kind(),
                Kind::ImportSpecifier | Kind::ImportClause | Kind::NamespaceImport | Kind::ImportEqualsDeclaration
            )
        })
    }

    fn call_by_op(&self, cx: &mut Cx<'_>, call: &CallSite, op: &str) -> PResult<Claim<LuauExpr>> {
        let arg = |i: usize| call.args.get(i).copied();
        match op {
            "hoist" => {
                let value = cx.compile_expr(arg(0).expect("an argument"))?;
                let hoisted = cx.hoist(value, "hoisted");
                self.claimed(hoisted)
            }
            "prereq" => {
                let value = cx.compile_expr(arg(0).expect("an argument"))?;
                cx.comment(" probe: prereq");
                let bound = cx.push_to_var(value, "probed");
                self.claimed(bound)
            }
            "order" => {
                let values = cx.compile_in_order(call.args)?;
                self.claimed(create::array(values))
            }
            "inject" => {
                let value = cx.compile_call(
                    call,
                    Callee::Original,
                    vec![Arg::Original(arg(0).expect("an argument")), Arg::Luau(text_literal("injected")), Arg::Nil],
                )?;
                self.claimed(value)
            }
            "swap" => {
                let function = cx.import_named("./runtime", "impl")?;
                let args =
                    vec![Arg::Original(arg(1).expect("two arguments")), Arg::Original(arg(0).expect("an argument"))];
                let value = cx.compile_call(call, Callee::Function(function), args)?;
                self.claimed(value)
            }
            "late" => {
                let key = format!("{}:{}", cx.file().relative_path, self.late_keys.borrow().len());
                self.late_keys.borrow_mut().push(key.clone());
                let value = cx.late_string(&key);
                self.claimed(value)
            }
            "capture" => {
                let (value, mut statements) = cx.capture(|cx| cx.compile_expr(arg(0).expect("an argument")))?;
                statements.push(create::return_stmt(value));
                self.claimed(create::call(create::paren(create::function(Vec::new(), false, statements)), Vec::new()))
            }
            "root" => {
                cx.insert_before_root_statement([create::comment(" probe: before the root statement")]);
                if !self.header_done.replace(true) {
                    cx.header([create::comment(" probe: header")]);
                }
                self.claimed(create::nil())
            }
            "elide" | "keep" => {
                let name = cx.source_text(call.callee).to_owned();
                if let Some(alias) = Self::alias_of(cx, call.callee) {
                    cx.request_import_elision(alias);
                    if op == "keep" {
                        cx.mark_import_used(alias);
                    }
                }
                self.claimed(text_literal(&name))
            }
            "new" if call.kind == CallKind::New => {
                let mut args = vec![Arg::Luau(text_literal("probed"))];
                args.extend(call.args.iter().map(|&a| Arg::Original(a)));
                let value = cx.compile_call(call, Callee::Original, args)?;
                self.claimed(value)
            }
            "decorator" if call.kind == CallKind::Decorator => {
                let value = cx.compile_call(call, Callee::Original, vec![Arg::Luau(text_literal("probed"))])?;
                self.claimed(value)
            }
            "facts" => {
                let pos = cx.utf16_pos(call.node);
                let width = cx.utf16_width(call.node);
                let symbol = call.symbol.expect("a callee symbol");
                let package = cx.declaring_package(symbol).and_then(|p| p.name.clone()).unwrap_or_default();
                let source = cx.file().source;
                let resolved = cx
                    .resolve_module("./runtime", source)
                    .map(|r| r.file_name.rsplit('/').next().unwrap_or_default().to_owned())
                    .unwrap_or_default();
                let tags = cx.symbol_jsdoc_tags(symbol);
                let link = tags.iter().find(|t| t.name == "probe").and_then(|t| t.links.first()).map(|link| {
                    let target = link.symbol.map_or("?", |s| s.name());
                    format!("{}={target}", cx.source_text(link.name))
                });
                let diagnostics = cx.file_diagnostics().len();
                let text = format!(
                    "{}:{} w{width} {package} {resolved} link:{} diagnostics:{diagnostics}",
                    pos.line + 1,
                    pos.character + 1,
                    link.unwrap_or_default()
                );
                self.claimed(text_literal(&text))
            }
            "self" => {
                let value = cx.compile_expr(call.node)?;
                self.claimed(create::call(create::id("wrap"), vec![value]))
            }
            "declineafter" => {
                cx.warning(call.node, "probe: declineafter was offered");
                cx.compile_expr(call.node)?;
                Ok(Claim::Decline)
            }
            "hoistdecline" => {
                cx.hoist(create::nil(), "declined");
                Ok(Claim::Decline)
            }
            "argdecline" => {
                // a nested hook (a hoist) runs inside compile_expr of the argument, then the call is declined
                cx.compile_expr(arg(0).expect("an argument"))?;
                Ok(Claim::Decline)
            }
            "approve" | "approveimpl" => {
                // declares a local the compiler approved, then compiles the argument
                let name = if op == "approve" { "_hoisted" } else { "impl" };
                if cx.is_usable_local_name(name) {
                    cx.prereq(create::local(create::any_id(name), Some(create::number(99.0))));
                }
                let value = cx.compile_expr(arg(0).expect("an argument"))?;
                self.claimed(value)
            }
            "inorderself" => {
                // the claimed node itself through compile_in_order: lowered as roblox-ts lowers it, without recursion
                let mut values = cx.compile_in_order(&[call.node])?;
                self.claimed(create::call(create::id("wrap"), vec![values.remove(0)]))
            }
            "latedecline" => {
                cx.late_string("declined");
                Ok(Claim::Decline)
            }
            "twice" => {
                // the claimed call compiled twice: the second is an internal error
                let first = cx.compile_expr(call.node)?;
                let second = cx.compile_expr(call.node)?;
                self.claimed(create::call(create::id("pair"), vec![first, second]))
            }
            "markdecline" => {
                if let Some(alias) = arg(0).and_then(|a| Self::alias_of(cx, a)) {
                    cx.mark_import_used(alias);
                }
                Ok(Claim::Decline)
            }
            "caprecv" => {
                // the receiver compiled inside a capture whose statements are dropped, then declined: an error
                let _ = cx.capture(|cx| cx.compile_receiver(call))?;
                Ok(Claim::Decline)
            }
            "decoself" if call.kind == CallKind::Decorator => {
                let value = cx.compile_expr(call.node)?;
                self.claimed(value)
            }
            "prereqdecline" => {
                cx.prereq(create::comment(" probe: left behind"));
                Ok(Claim::Decline)
            }
            "recvarg" => {
                // the receiver as an argument: compiled once (a non-method optional call binds it to a local)
                let receiver = cx.compile_receiver(call)?;
                let mut args = vec![Arg::Luau(receiver)];
                args.extend(call.args.iter().map(|&a| Arg::Original(a)));
                let value = cx.compile_call(call, Callee::Function(create::id("useRecv")), args)?;
                self.claimed(value)
            }
            "recvsuper" => {
                // `super` is not a value: an internal error
                let receiver = cx.compile_receiver(call)?;
                self.claimed(receiver)
            }
            "objedit" => {
                let edits = ObjectEdits {
                    rename: vec![("a".to_owned(), text_literal("renamed"))],
                    replace: vec![("b".to_owned(), text_literal("replaced"))],
                    append: vec![(text_literal("e"), create::number(5.0))],
                    order: FieldOrder::Source,
                };
                let value = cx.compile_object_literal(arg(0).expect("an argument"), edits)?;
                self.claimed(create::call(create::id("wrap"), vec![value]))
            }
            "objorder" => {
                let edits = ObjectEdits {
                    append: vec![(text_literal("w"), create::number(4.0))],
                    order: FieldOrder::Permuted(vec![3, 2, 0, 1]),
                    ..ObjectEdits::default()
                };
                let value = cx.compile_object_literal(arg(0).expect("an argument"), edits)?;
                self.claimed(value)
            }
            "binary" => {
                let value =
                    cx.compile_binary(Kind::PlusToken, arg(0).expect("an argument"), arg(1).expect("two arguments"))?;
                self.claimed(value)
            }
            "locals" => {
                let locals = cx.locals();
                self.claimed(text_literal(&format!("f{} m{}", locals.function, locals.main_chunk)))
            }
            "spill" => {
                let value = cx.compile_expr(arg(0).expect("an argument"))?;
                let hoisted = cx.hoist_spillable(value, "spilled");
                self.claimed(hoisted)
            }
            "calltwice" => {
                let args = || call.args.iter().map(|&a| Arg::Original(a)).collect::<Vec<_>>();
                cx.compile_call(call, Callee::Original, args())?;
                let value = cx.compile_call(call, Callee::Original, args())?;
                self.claimed(value)
            }
            "exprthencall" => {
                cx.compile_expr(call.node)?;
                let value =
                    cx.compile_call(call, Callee::Original, call.args.iter().map(|&a| Arg::Original(a)).collect())?;
                self.claimed(value)
            }
            "ancestor" => {
                let parent = call.node.parent().expect("a call has a parent");
                let value = cx.compile_expr(parent)?;
                self.claimed(value)
            }
            "binbad" => {
                let value = cx.compile_binary(
                    Kind::PlusEqualsToken,
                    arg(0).expect("an argument"),
                    arg(1).expect("two arguments"),
                )?;
                self.claimed(value)
            }
            "recvexpr" => {
                // the receiver node itself: compiled already for a call after `?.` (an internal error)
                let receiver = cx.compile_expr(call.receiver.expect("a receiver"))?;
                let value = cx.compile_call(
                    call,
                    Callee::Method { object: receiver, name: "re".to_owned() },
                    call.args.iter().map(|&a| Arg::Original(a)).collect(),
                )?;
                self.claimed(value)
            }
            "boom" if self.mode == Mode::Panic => panic!("probe: boom"),
            "fail" if self.mode == Mode::Fail => Err(cx.error(call.node, "probe: fail was called")),
            "err" if self.mode == Mode::ErrWithoutError => {
                let saved = self.saved.borrow_mut().take();
                match saved {
                    Some(abort) => Err(abort),
                    None => {
                        let abort = cx.error(call.node, "probe: the first err");
                        *self.saved.borrow_mut() = Some(abort);
                        Ok(Claim::Decline)
                    }
                }
            }
            _ => Ok(Claim::Decline),
        }
    }
}

impl PluginInstance for ProbeInstance {
    type FilePart = ProbePart;

    fn interest(&self, interest: &mut Interest) {
        if self.mode == Mode::ExprCall {
            interest.expressions(Kind::CallExpression, EXPRCALL);
            interest.expressions(Kind::NewExpression, EXPRCALL);
            interest.expressions(Kind::BinaryExpression, EXPRBIN);
            interest.expressions(Kind::PostfixUnaryExpression, EXPRBIN);
            interest.statements(Kind::ExpressionStatement, STMTSELF);
            return;
        }
        if self.mode == Mode::ClassSelf {
            interest.classes(ClassFilter::All);
            return;
        }
        interest.calls_by_symbol();
        if self.mode == Mode::DeclineTouched {
            return;
        }
        interest.calls_by_receiver(&["probeMethod"], "__probeMarker", RECEIVER);
        interest.calls_by_callee_marker("__probeCallee", CALLEE);
        interest.classes(ClassFilter::Decorated);
        interest.expressions(Kind::VoidExpression, VOID);
        interest.statements(Kind::DebuggerStatement, DEBUGGER);
        interest.accesses_by_marker("__probeKey", KEY);
        interest.assignments_by_receiver_symbol();
    }

    fn classify_symbol(&self, cx: &mut Cx<'_>, symbol: P<Symbol>, role: SymbolRole) -> Option<Tag> {
        if self.mode == Mode::Refuse {
            cx.late_string("refused in classify_symbol");
        }
        if role == SymbolRole::AssignReceiver {
            let tags = cx.symbol_jsdoc_tags(symbol);
            return tags.iter().find_map(|t| match t.name {
                "probeattrs" => Some(ATTRS),
                "probeattrsread" => Some(ATTRSREAD),
                _ => None,
            });
        }
        let tag = cx.symbol_jsdoc_tags(symbol).into_iter().find(|t| t.name == "probe")?;
        let op = tag.comment.split_whitespace().next().unwrap_or("");
        OPS.iter().position(|&o| o == op).map(|i| Tag(i as u32))
    }

    fn begin_file(&self, cx: &mut Cx<'_>) -> PResult<()> {
        self.claims.set(0);
        self.late_keys.borrow_mut().clear();
        self.header_done.set(false);
        if self.mode == Mode::Refuse {
            cx.prereq(create::comment("refused in begin_file"));
        }
        Ok(())
    }

    fn call(&self, cx: &mut Cx<'_>, call: &CallSite, tag: Tag) -> PResult<Claim<LuauExpr>> {
        if self.mode == Mode::DeclineTouched {
            // a query that records a name: it changes later naming, so it touches compiler state
            cx.is_usable_local_name("touchedName");
            return Ok(Claim::Decline);
        }
        match tag {
            RECEIVER => {
                let object = cx.compile_receiver(call)?;
                let args = call.args.iter().map(|&a| Arg::Original(a)).collect();
                let value = cx.compile_call(call, Callee::Method { object, name: "probed".to_owned() }, args)?;
                self.claimed(value)
            }
            CALLEE => {
                let args = vec![Arg::Original(call.args[0]), Arg::Luau(create::number(1.0))];
                let value = cx.compile_call(call, Callee::Original, args)?;
                self.claimed(value)
            }
            tag => self.call_by_op(cx, call, op(tag)),
        }
    }

    fn class(&self, cx: &mut Cx<'_>, class: &ClassSite) -> PResult<Vec<LuauStmt>> {
        if self.mode == Mode::ClassSelf {
            // a class expression compiled by its own class hook: an internal error
            if class.node.kind() == Kind::ClassExpression {
                cx.compile_expr(class.node)?;
            }
            return Ok(Vec::new());
        }
        Ok(vec![create::assignment(
            create::property(class.luau_name.clone(), "__probed"),
            AssignmentOperator::Assign,
            text_literal(self.name),
        )])
    }

    fn expression(&self, cx: &mut Cx<'_>, node: P<Node>, tag: Tag) -> PResult<Claim<LuauExpr>> {
        if tag == EXPRBIN {
            // assignments to `probeTarget`, lowered by the hook itself: as a statement where roblox-ts lowers them as
            // one (the value is `nil`), else wrapped
            if !cx.source_text(node).starts_with("probeTarget") {
                return Ok(Claim::Decline);
            }
            let value = cx.compile_expr(node)?;
            let statement = matches!(value, LuauExpr::Nil);
            return Ok(Claim::Take(if statement { value } else { create::call(create::id("exprWrap"), vec![value]) }));
        }
        if tag == EXPRCALL
            && let Some(callee) = node.expression()
            && cx.source_text(callee).ends_with(".probeExprBase")
            && let Some(base) = callee.expression()
        {
            // the base of a call offered inside a chain is compiled already (an internal error)
            let value = cx.compile_expr(base)?;
            return self.claimed(value);
        }
        if tag == EXPRCALL
            && let Some(callee) = node.expression()
            && cx.source_text(callee).starts_with("probeAskOnce")
        {
            cx.warning(node, "probe-exprcall: asked about probeAskOnce");
            return Ok(Claim::Decline);
        }
        if tag == EXPRCALL
            && let Some(callee) = node.expression()
            && (cx.source_text(callee).starts_with("probeExpr")
                || cx.source_text(callee).starts_with("ProbeExpr")
                || cx.source_text(callee).contains(".probeExpr"))
        {
            let value = cx.compile_expr(node)?;
            return self.claimed(create::call(create::id("exprWrap"), vec![value]));
        }
        if tag == VOID
            && let Some(operand) = node.expression()
        {
            match cx.source_text(operand) {
                "probeVoid" => return self.claimed(text_literal("void claimed")),
                // declined after a hoist: an internal error (used by the tests only)
                "probeVoidHoist" => {
                    cx.hoist(create::nil(), "void");
                }
                // compiles its own node (whose operand is dispatched to this hook too), then declines: an error
                "void probeVoidSelf" => {
                    cx.compile_expr(node)?;
                }
                // its own node through compile_in_order: lowered as roblox-ts lowers it
                "probeVoidInOrder" => {
                    let mut values = cx.compile_in_order(&[node])?;
                    return self.claimed(create::call(create::id("wrap"), vec![values.remove(0)]));
                }
                _ => {}
            }
        }
        Ok(Claim::Decline)
    }

    fn statement(&self, cx: &mut Cx<'_>, node: P<Node>, tag: Tag) -> PResult<Claim<Vec<LuauStmt>>> {
        if tag == STMTSELF && cx.source_text(node).starts_with("probeStmtSelf") {
            cx.compile_expr(node)?;
            return Ok(Claim::Decline);
        }
        if tag != DEBUGGER {
            return Ok(Claim::Decline);
        }
        self.claims.set(self.claims.get() + 1);
        let print = create::call(create::id("print"), vec![text_literal("probe: debugger")]);
        Ok(Claim::Take(vec![create::call_statement(print)]))
    }

    fn property_key(&self, _: &mut Cx<'_>, access: &AccessSite, tag: Tag) -> PResult<Claim<String>> {
        if tag != KEY {
            return Ok(Claim::Decline);
        }
        Ok(Claim::Take(match access.key {
            "swap" => "key with space".to_owned(),
            "same" => "same".to_owned(),
            key => format!("k_{key}"),
        }))
    }

    fn assignment(&self, cx: &mut Cx<'_>, site: &AssignSite, tag: Tag) -> PResult<Claim<LuauExpr>> {
        // `this.attrs.key op= value` → `self:setAttribute("key", value[, true])`, as Flamework's attribute setter
        let Some(this) = (site.receiver.kind() == Kind::PropertyAccessExpression)
            .then(|| site.receiver.as_property_access_expression().expression)
        else {
            return Err(cx.error(site.target, "probe: an attribute assignment needs `this.attrs.key`"));
        };
        let object = cx.compile_expr(this)?;
        let one = || create::number(1.0);
        let value = match (site.op, tag) {
            (_, ATTRSREAD) => {
                // reads the target, whatever the operation (a `delete` operand is refused)
                let target = cx.compile_expr(site.target)?;
                create::call(create::id("read"), vec![target])
            }
            (AssignOp::Assign, _) => cx.compile_expr(site.value.expect("an assignment has a value"))?,
            (AssignOp::Compound(op), _) => cx.compile_binary(op, site.target, site.value.expect("a value"))?,
            (AssignOp::PreInc | AssignOp::PostInc, _) => {
                create::binary(cx.compile_expr(site.target)?, BinaryOperator::Add, one())
            }
            (AssignOp::PreDec | AssignOp::PostDec, _) => {
                create::binary(cx.compile_expr(site.target)?, BinaryOperator::Sub, one())
            }
            (AssignOp::Delete, _) => create::nil(),
        };
        let mut args = vec![text_literal(site.key), value];
        if matches!(site.op, AssignOp::PostInc | AssignOp::PostDec) && site.value_used {
            args.push(create::bool(true));
        }
        self.claims.set(self.claims.get() + 1);
        Ok(Claim::Take(create::method_call(object, "setAttribute", args)))
    }

    fn explain_checker_diagnostic(&self, cx: &mut Cx<'_>, diagnostic: P<Diagnostic>) -> Option<PluginDiagnostic> {
        let text = diagnostic.localize();
        text.contains("probeBroken").then(|| {
            PluginDiagnostic::error(cx.file().source.as_node(), format!("probe explained: {text}"))
                .with_hint("Declare probeBroken first.")
        })
    }

    fn end_file(&self, cx: &mut Cx<'_>) -> PResult<ProbePart> {
        if self.mode == Mode::Refuse {
            cx.hoist(create::nil(), "refused");
        }
        Ok(ProbePart {
            file: cx.file().relative_path.clone(),
            claims: self.claims.replace(0),
            late_keys: std::mem::take(&mut *self.late_keys.borrow_mut()),
        })
    }
}
