//! The guard generator: `createGuardGenerator` of `buildGuardFromType.ts`, emitting Luau.
//!
//! The Luau is what roblox-ts makes of the TS builder's output: `t.*` calls, `function(value) … end` for the
//! checks the builder writes as arrow functions, `type(x) == "table"` for `typeIs`, `memo[k]` / `memo[k] = v` for the
//! `Map` macros, `for key in pairs(value)` and numeric `for` loops, and element accesses with roblox-ts's `+ 1`
//! offset folded in (`list[1]`, `list[index]`, `list[size - 1]`). Locals are placeholders until the guard is complete
//! ([`super::names`]).

use std::collections::{HashMap, HashSet};

use rbxts_luau::{AssignmentOperator, BinaryOperator, ElseBody, Expr, Stmt, UnaryOperator, create};
use rbxts_plugin::ts::{Node, P, Type};
use rbxts_plugin::{Abort, Cx, PResult, PluginDiagnostic};
use tsrs_checker::{ElementFlags, TypeFlags};

use super::names::{Names, declared, local_name, placeholder_index, ts_symbol_name};
use super::types::{
    alias_symbol, declaration_of_type, extract_types, get_literal, global_type, has_call_signatures,
    instance_type_of, is_class_or_interface, text_literal, is_instance_type, is_object, key_expr, property_key, simplify_union,
};

/// `GUARD_RECURSION_LIMIT`: how many times one check may go round a recursive type into a value before it refuses
/// the value.
pub const GUARD_RECURSION_LIMIT: u32 = 1000;

/// `TYPE_NESTING_LIMIT`: how many types deep a guard's type may nest before the build gives up on it.
const TYPE_NESTING_LIMIT: usize = 500;

/// `GUARD_LOCALS` (= `COMPACT_LOCALS`): how many guards a function that holds them keeps in locals of its own before
/// it keeps them in a table.
const GUARD_LOCALS: usize = 150;

/// `DEDUP_HEURISTIC_LIMIT`
pub const DEDUP_HEURISTIC_LIMIT: u32 = 5;

/// Types that compile to `t.typeof("<name>")` (`RBX_TYPES_NEW`): userdata `t` has no alias for.
const RBX_TYPES_NEW: [&str; 2] = ["buffer", "InstanceHandle"];

/// `RBX_TYPES`, in the TS order (the first match wins).
const RBX_TYPES: [&str; 36] = [
    "UDim",
    "UDim2",
    "BrickColor",
    "Color3",
    "Vector2",
    "Vector3",
    "NumberSequence",
    "NumberSequenceKeypoint",
    "ColorSequence",
    "ColorSequenceKeypoint",
    "NumberRange",
    "Rect",
    "DockWidgetPluginGuiInfo",
    "CFrame",
    "Axes",
    "Faces",
    "Font",
    "Instance",
    "Ray",
    "Random",
    "Region3",
    "Region3int16",
    "Enum",
    "TweenInfo",
    "PhysicalProperties",
    "Vector3int16",
    "Vector2int16",
    "PathWaypoint",
    "EnumItem",
    "RBXScriptSignal",
    "RBXScriptConnection",
    "FloatCurveKey",
    "OverlapParams",
    "thread",
    "buffer",
    "InstanceHandle",
];

/// A type whose guard is being built, with the types it is being built inside of below it (`GuardFrame`).
struct Frame {
    ty: P<Type>,
    /// Its place among the open frames, counted from the outermost.
    index: usize,
    /// The local its guard is kept in, given once something inside it reaches the type again.
    self_id: Option<Expr>,
}

/// Where the guards of recursive types are kept (`GuardScope`).
#[derive(Default)]
struct Scope {
    /// How many frames were open when the scope began: a frame below this is outside it.
    base: usize,
    locals: Vec<Expr>,
    /// The assignments of `locals`, in the order their guards were finished.
    statements: Vec<Stmt>,
    /// The recursive types finished in this scope, by the local holding each one's guard.
    finished: HashMap<P<Type>, Expr>,
    /// A guard built in it calls one still being built outside it.
    escapes: bool,
}

/// A guard the dedup heuristic declared once: `local <id> = <value>`.
pub(crate) struct Dedup {
    pub id: Expr,
    pub value: Expr,
}

/// What the recursive guards share while a check runs (`CheckState`).
#[derive(Clone)]
struct CheckState {
    depth: Expr,
    memo: Expr,
}

/// One guard generator (`createGuardGenerator`): one per guard the TS builds, so locals, the dedup set and the
/// recursion state are the guard's own.
pub(crate) struct Generator<'g, 'x> {
    pub(crate) cx: &'g mut Cx<'x>,
    library: String,
    library_warning: Option<&'static str>,
    t: Option<Expr>,
    diagnostic_node: P<Node>,
    pub(crate) names: Names,
    tracking: Vec<(P<Node>, P<Type>)>,
    pub(crate) dedup: Vec<Dedup>,
    dedup_ids: HashMap<P<Type>, Expr>,
    requires_dedup: HashSet<P<Type>>,
    frames: Vec<Frame>,
    open_frames: HashMap<P<Type>, usize>,
    scopes: Vec<Scope>,
    check_state: Option<CheckState>,
    dedup_is_recursive: bool,
    rbx_symbols: Option<Vec<Option<P<tsrs_ast::Symbol>>>>,
}

impl<'g, 'x> Generator<'g, 'x> {
    pub(crate) fn new(cx: &'g mut Cx<'x>, package_name: &str, diagnostic_node: P<Node>) -> Self {
        let (library, library_warning) = super::library::guard_library(cx, package_name);
        Generator {
            cx,
            library,
            library_warning,
            t: None,
            diagnostic_node,
            names: Names::default(),
            tracking: Vec::new(),
            dedup: Vec::new(),
            dedup_ids: HashMap::new(),
            requires_dedup: HashSet::new(),
            frames: Vec::new(),
            open_frames: HashMap::new(),
            scopes: Vec::new(),
            check_state: None,
            dedup_is_recursive: false,
            rbx_symbols: None,
        }
    }

    // --- diagnostics ---

    /// `fail`: an error at the guard's node, with a "Type was defined here" note for each type the guard was being
    /// built inside of (a run of the same type once).
    fn fail(&mut self, message: impl Into<String>) -> Abort {
        let mut diagnostic = PluginDiagnostic::error(self.diagnostic_node, message);
        let mut previous: Option<P<Type>> = None;
        for &(node, ty) in &self.tracking.clone() {
            if previous == Some(ty) {
                continue;
            }
            previous = Some(ty);
            let at = node.name().unwrap_or(node);
            let text = self.cx.checker().type_to_string_exported(ty);
            diagnostic = diagnostic.with_related(at, format!("Type was defined here: {text}"));
        }
        self.cx.report(diagnostic).expect("an error")
    }

    // --- `t` ---

    /// The guard library's `t`, imported at its first use (an unused import is left out, as roblox-ts elides one).
    fn t(&mut self) -> PResult<Expr> {
        if let Some(t) = &self.t {
            return Ok(t.clone());
        }
        if let Some(warning) = self.library_warning.take() {
            let end = self.cx.file().source.get().end_of_file_token();
            self.cx.warning(end, warning);
        }
        let t = self.cx.import_named(&self.library.clone(), "t")?;
        self.t = Some(t.clone());
        Ok(t)
    }

    /// `t.<name>`
    fn t_field(&mut self, name: &str) -> PResult<Expr> {
        Ok(create::property(self.t()?, name))
    }

    /// `t.<name>(args)`
    fn t_call(&mut self, name: &str, args: Vec<Expr>) -> PResult<Expr> {
        Ok(create::call(self.t_field(name)?, args))
    }

    /// `listLikeGuard`: `t.<kind>(a, b)` for up to two members, `t.<kind>List({ … })` beyond (a Luau call has a
    /// hard limit on its argument count).
    fn list_like(&mut self, kind: &str, list: Vec<Expr>) -> PResult<Expr> {
        if list.len() <= 2 {
            return self.t_call(kind, list);
        }
        self.t_call(&format!("{kind}List"), vec![create::array(list)])
    }

    // --- dedup ---

    /// `calculateDedup`
    pub(crate) fn calculate_dedup(&mut self, ty: P<Type>, limit: u32) {
        self.requires_dedup = types_requiring_dedup(self.cx, ty, limit);
    }

    // --- building ---

    /// `buildGuard`
    pub(crate) fn build_guard(&mut self, ty: P<Type>) -> PResult<Expr> {
        // A guard of its own: the outermost scope declares what it needs.
        if self.scopes.is_empty() {
            self.open_scope();
            let guard = self.build_guard(ty)?;
            let root = self.scopes.pop().expect("the root scope");
            return self.close_scope(root, guard, true);
        }

        // The type is reached again inside its own guard.
        if let Some(&frame) = self.open_frames.get(&ty) {
            return self.call_recursive(frame);
        }

        if let Some(finished) = self.scopes.last().and_then(|s| s.finished.get(&ty)) {
            return Ok(finished.clone());
        }

        if self.requires_dedup.contains(&ty) {
            if let Some(existing) = self.dedup_ids.get(&ty) {
                return Ok(existing.clone());
            }
            return self.build_dedup_guard(ty);
        }

        self.build_frame(ty)
    }

    /// `buildDedupGuard`: a guard the dedup heuristic picked, declared once and referenced.
    fn build_dedup_guard(&mut self, ty: P<Type>) -> PResult<Expr> {
        self.open_scope();
        let guard = self.build_frame(ty)?;
        let scope = self.scopes.pop().expect("the dedup scope");

        if scope.escapes {
            let outer = self.scopes.last_mut().expect("A guard escaped the outermost scope");
            outer.locals.extend(scope.locals);
            outer.statements.extend(scope.statements);
            outer.finished.extend(scope.finished);
            return Ok(guard);
        }

        let type_name = alias_symbol(ty)
            .or_else(|| ty.symbol())
            .map_or_else(|| "dedup".to_owned(), |s| ts_symbol_name(s.name()).into_owned());
        let name = local_name(self.cx, &type_name);
        let id = self.names.fresh(name);
        self.dedup_ids.insert(ty, id.clone());
        let value = self.close_scope(scope, guard, false)?;
        self.dedup.push(Dedup { id: id.clone(), value });
        Ok(id)
    }

    fn open_scope(&mut self) {
        self.scopes.push(Scope { base: self.frames.len(), ..Scope::default() });
    }

    /// `buildFrame`: the guard of a type, as a frame other guards built inside it can come back to.
    fn build_frame(&mut self, ty: P<Type>) -> PResult<Expr> {
        if self.frames.len() >= TYPE_NESTING_LIMIT {
            return Err(self.fail(format!(
                "Flamework cannot generate a guard for a type nested over {TYPE_NESTING_LIMIT} types deep. A type that makes a new type of itself as it nests (`Box<Box<T>>`) never ends."
            )));
        }
        assert!(!self.scopes.is_empty(), "A guard frame was built outside a scope");

        let index = self.frames.len();
        self.frames.push(Frame { ty, index, self_id: None });
        self.open_frames.insert(ty, index);
        let result = self.build_tracked_guard(ty);
        let frame = self.frames.pop().expect("the frame");
        self.open_frames.remove(&ty);
        let guard = result?;

        let Some(self_id) = frame.self_id else { return Ok(guard) };
        // Kept in its local, assigned before anything after it in the scope can use it.
        let scope = self.scopes.last_mut().expect("a scope");
        scope.locals.push(self_id.clone());
        scope.statements.push(create::assignment(self_id.clone(), AssignmentOperator::Assign, guard));
        scope.finished.insert(ty, self_id.clone());
        Ok(self_id)
    }

    fn build_tracked_guard(&mut self, ty: P<Type>) -> PResult<Expr> {
        let declaration = declaration_of_type(ty);
        if let Some(declaration) = declaration {
            self.tracking.push((declaration, ty));
        }
        let guard = self.build_guard_inner(ty)?;
        if declaration.is_some() {
            self.tracking.pop();
        }
        Ok(guard)
    }

    /// The count and the memo of the recursive guards (`getCheckState`).
    fn check_state(&mut self) -> CheckState {
        if self.check_state.is_none() {
            let depth = self.names.fresh("depth");
            let memo = self.names.fresh("memo");
            self.check_state = Some(CheckState { depth, memo });
        }
        self.check_state.clone().expect("set above")
    }

    /// `callRecursive`: a type reached inside its own guard, checked through a function that refuses the value past
    /// [`GUARD_RECURSION_LIMIT`] levels and remembers what the guard said of each table.
    fn call_recursive(&mut self, frame: usize) -> PResult<Expr> {
        let self_id = match &self.frames[frame].self_id {
            Some(id) => id.clone(),
            None => {
                let name = self.guard_name(self.frames[frame].ty, "recursive");
                let name = local_name(self.cx, &name);
                let id = self.names.fresh(name);
                self.frames[frame].self_id = Some(id.clone());
                id
            }
        };

        // A scope that began above the frame cannot leave the scope the frame is in.
        let base = self.frames[frame].index;
        for scope in &mut self.scopes {
            if scope.base > base {
                scope.escapes = true;
            }
        }

        let CheckState { depth, memo } = self.check_state();
        let value = self.names.fresh("value");
        let results = self.names.fresh("results");
        let known = self.names.fresh("known");
        let result = self.names.fresh("result");
        let assign = |target: &Expr, value: Expr| create::assignment(target.clone(), AssignmentOperator::Assign, value);

        let body = vec![
            create::if_stmt(
                create::binary(depth.clone(), BinaryOperator::Ge, create::number(f64::from(GUARD_RECURSION_LIMIT))),
                vec![create::return_stmt(create::bool(false))],
                ElseBody::default(),
            ),
            // Only a table is remembered: a check of anything else goes no deeper.
            create::local(declared(&results), None),
            create::if_stmt(
                type_is(value.clone(), "table"),
                vec![
                    assign(&results, create::computed_index(memo.clone(), self_id.clone())),
                    create::if_stmt(
                        create::binary(results.clone(), BinaryOperator::Eq, create::nil()),
                        vec![
                            assign(&results, create::map(Vec::new())),
                            assign(&create::computed_index(memo.clone(), self_id.clone()), results.clone()),
                        ],
                        ElseBody::Block(vec![
                            create::local(declared(&known), Some(create::computed_index(results.clone(), value.clone()))),
                            create::if_stmt(
                                create::binary(known.clone(), BinaryOperator::Ne, create::nil()),
                                vec![create::return_stmt(known.clone())],
                                ElseBody::default(),
                            ),
                        ]),
                    ),
                ],
                ElseBody::default(),
            ),
            create::assignment(depth.clone(), AssignmentOperator::Add, create::number(1.0)),
            create::local(declared(&result), Some(create::call(self_id.clone(), vec![value.clone()]))),
            create::assignment(depth, AssignmentOperator::Sub, create::number(1.0)),
            create::if_stmt(
                create::binary(results.clone(), BinaryOperator::Ne, create::nil()),
                vec![assign(&create::computed_index(results, value.clone()), result.clone())],
                ElseBody::default(),
            ),
            create::return_stmt(result),
        ];
        Ok(create::function(vec![declared(&value)], false, body))
    }

    /// `closeScope`: a guard built in `scope`. One holding recursive types is wrapped in a function called once,
    /// which declares their locals. A deduplicated guard's (not `root`) returns the guard; the guard's own declares
    /// the count and the memo first, then the deduplicated guards when one of them is recursive, and returns the
    /// function the guard is called through.
    fn close_scope(&mut self, scope: Scope, guard: Expr, root: bool) -> PResult<Expr> {
        if !root {
            if scope.locals.is_empty() {
                return Ok(guard);
            }
            self.dedup_is_recursive = true;
            let mut body = scope.statements;
            body.push(create::return_stmt(guard));
            let body = self.hold_guards(scope.locals, Vec::new(), body);
            return Ok(called_once(body));
        }

        // The deduplicated guards are only ever called from this one.
        let deduplicated = if self.dedup_is_recursive { std::mem::take(&mut self.dedup) } else { Vec::new() };
        if scope.locals.is_empty() && deduplicated.is_empty() {
            return Ok(guard);
        }

        let mut rest = scope.statements;
        let entry = if matches!(guard, Expr::Identifier(_)) {
            guard
        } else {
            let entry = self.names.fresh("guard");
            rest.push(create::local(declared(&entry), Some(guard)));
            entry
        };
        rest.push(create::return_stmt(self.check_through(entry)));

        let CheckState { depth, memo } = self.check_state();
        let mut body = vec![create::local(declared(&depth), Some(create::number(0.0))), create::local(declared(&memo), None)];
        body.extend(self.hold_guards(scope.locals, deduplicated, rest));
        Ok(called_once(body))
    }

    /// `holdGuards`: the body of a function holding guards: the deduplicated ones, the locals of the recursive ones,
    /// then `rest`. Past [`GUARD_LOCALS`] of them they are fields of one table (`guards.Name`) instead, Luau
    /// allowing a function 200 locals.
    fn hold_guards(&mut self, selves: Vec<Expr>, deduplicated: Vec<Dedup>, rest: Vec<Stmt>) -> Vec<Stmt> {
        if selves.len() + deduplicated.len() <= GUARD_LOCALS {
            let mut body: Vec<Stmt> =
                deduplicated.into_iter().map(|d| create::local(declared(&d.id), Some(d.value))).collect();
            body.extend(selves.iter().map(|id| create::local(declared(id), None)));
            body.extend(rest);
            return body;
        }

        let table = self.names.fresh("guards");
        let mut taken = HashSet::new();
        let mut replacements = HashMap::new();
        let mut field_of = |names: &Names, local: &Expr| -> String {
            let base = names.base_of(local).expect("a guard local").to_owned();
            let mut name = base.clone();
            let mut suffix = 1;
            while taken.contains(&name) {
                name = format!("{base}_{suffix}");
                suffix += 1;
            }
            taken.insert(name.clone());
            let index = placeholder_index(local).expect("a placeholder");
            replacements.insert(index, create::property(table.clone(), name.clone()));
            name
        };

        // Each deduplicated guard now assigns its field.
        let assigned: Vec<(String, Expr)> =
            deduplicated.into_iter().map(|d| (field_of(&self.names, &d.id), d.value)).collect();
        for id in &selves {
            field_of(&self.names, id);
        }

        let mut body = vec![create::local(declared(&table), Some(create::map(Vec::new())))];
        for (name, value) in assigned {
            body.push(create::assignment(create::property(table.clone(), name), AssignmentOperator::Assign, value));
        }
        body.extend(rest);
        Names::substitute(&mut body[1..], &replacements);
        body
    }

    /// `holdInFunction`: the guard and its deduplicated guards in a function of their own, called once.
    pub(crate) fn hold_in_function(&mut self, guard: Expr) -> Expr {
        let deduplicated = std::mem::take(&mut self.dedup);
        let body = self.hold_guards(Vec::new(), deduplicated, vec![create::return_stmt(guard)]);
        called_once(body)
    }

    /// `checkThrough`: the function a guard with recursive types is called through: a memo of its own for the
    /// check, and the count and the memo put back as they were, even when the check raised (raised again as it was).
    fn check_through(&mut self, guard: Expr) -> Expr {
        let CheckState { depth, memo } = self.check_state();
        let value = self.names.fresh("value");
        let outer_depth = self.names.fresh("outerDepth");
        let outer_memo = self.names.fresh("outerMemo");
        let ok = self.names.fresh("ok");
        let result = self.names.fresh("result");
        let body = vec![
            create::local(declared(&outer_depth), Some(depth.clone())),
            create::local(declared(&outer_memo), Some(memo.clone())),
            create::assignment(memo.clone(), AssignmentOperator::Assign, create::map(Vec::new())),
            create::locals(
                vec![declared(&ok), declared(&result)],
                vec![create::call(create::id("pcall"), vec![guard, value.clone()])],
            ),
            create::assignment(depth, AssignmentOperator::Assign, outer_depth),
            create::assignment(memo, AssignmentOperator::Assign, outer_memo),
            create::if_stmt(
                create::unary(UnaryOperator::Not, ok),
                vec![create::call_statement(create::call(create::id("error"), vec![result.clone(), create::number(0.0)]))],
                ElseBody::default(),
            ),
            create::return_stmt(result),
        ];
        create::function(vec![declared(&value)], false, body)
    }

    /// `guardName`: a name for the local of a recursive type's guard: the type's, `TreeArray` for `Tree[]`,
    /// `TreeOptional` for an optional property's `Tree | undefined`.
    fn guard_name(&mut self, ty: P<Type>, fallback: &str) -> String {
        if alias_symbol(ty).is_none() && self.cx.checker().is_array_type(ty) {
            let element = self.cx.checker().get_type_arguments(ty).first().copied();
            let name = match element {
                Some(element) => self.guard_name(element, fallback),
                None => fallback.to_owned(),
            };
            return format!("{name}Array");
        }

        if alias_symbol(ty).is_none() && ty.is_union() {
            let (optional, members) = extract_types(self.cx, ty.types());
            if optional && members.len() == 1 {
                let name = self.guard_name(members[0], fallback);
                return if name == fallback { fallback.to_owned() } else { format!("{name}Optional") };
            }
        }

        match alias_symbol(ty).or_else(|| ty.symbol()) {
            Some(symbol) => {
                let name = ts_symbol_name(symbol.name());
                if name == "__type" || name == "__object" { fallback.to_owned() } else { name.into_owned() }
            }
            None => fallback.to_owned(),
        }
    }

    /// The global type symbols of [`RBX_TYPES`], resolved once per generator.
    fn rbx_symbols(&mut self) -> Vec<Option<P<tsrs_ast::Symbol>>> {
        if self.rbx_symbols.is_none() {
            let symbols = RBX_TYPES.iter().map(|name| global_type(self.cx, name)).collect();
            self.rbx_symbols = Some(symbols);
        }
        self.rbx_symbols.clone().expect("set above")
    }

    /// `buildGuardInner`
    fn build_guard_inner(&mut self, ty: P<Type>) -> PResult<Expr> {
        let flags = ty.flags();

        if ty.is_union() {
            return self.build_union_guard(ty);
        }

        if is_instance_type(self.cx, ty) {
            return self.build_instance_guard(ty);
        }

        if ty.is_intersection() {
            return self.build_intersection_guard(ty);
        }

        if flags.intersects(TypeFlags::Conditional) {
            let true_type = self.cx.checker().get_true_type_of_conditional_type(ty);
            let false_type = self.cx.checker().get_false_type_of_conditional_type(ty);
            let guards = vec![self.build_guard(true_type)?, self.build_guard(false_type)?];
            return self.list_like("union", guards);
        }

        if flags.intersects(TypeFlags::TypeVariable) {
            let Some(constraint) = self.cx.checker().get_base_constraint_of_type(ty) else {
                return Err(self.fail("could not find constraint of type parameter"));
            };
            return self.build_guard(constraint);
        }

        if let Some(literals) = get_literal(self.cx, ty, false) {
            return self.list_like("literal", literals);
        }

        if tsrs_checker::is_tuple_type_exported(ty) {
            let arguments = self.cx.checker().get_type_arguments(ty);
            let target = ty.target_tuple_type();
            let element_flags = target.element_flags();
            if let Some(rest) = element_flags.iter().position(|f| f.intersects(ElementFlags::Rest)) {
                let min_length = target.min_length.get();
                return self.build_rest_tuple_guard(
                    &arguments[..rest.min(arguments.len())],
                    arguments.get(rest).copied(),
                    arguments.get(rest + 1..).unwrap_or(&[]),
                    min_length,
                );
            }
            let mut guards = Vec::new();
            for &argument in arguments {
                guards.push(self.build_guard(argument)?);
            }
            return self.t_call("strictArray", guards);
        }

        if self.cx.checker().is_array_type(ty) {
            let element = self.cx.checker().get_type_arguments(ty).first().copied();
            let guard = match element {
                Some(element) => self.build_guard(element)?,
                None => self.t_field("any")?,
            };
            return self.t_call("array", vec![guard]);
        }

        if has_call_signatures(self.cx, ty) {
            return self.t_field("callback");
        }

        let checker = self.cx.checker();
        if ty == checker.get_void_type() || ty == checker.get_undefined_type() {
            return self.t_field("none");
        }
        if ty == checker.get_any_type() {
            return self.t_field("any");
        }
        if ty == checker.get_string_type() {
            return self.t_field("string");
        }
        if ty == checker.get_number_type() {
            return self.t_field("number");
        }

        if flags.intersects(TypeFlags::Unknown) {
            let guards = vec![self.t_field("any")?, self.t_field("none")?];
            return self.list_like("union", guards);
        }

        if flags.intersects(TypeFlags::TemplateLiteral) {
            // `${string}-id` becomes an anchored Lua pattern: the literal parts verbatim, each placeholder matching
            // anything.
            let texts = ty.as_template_literal_type().texts();
            let pattern = format!("^{}$", texts.iter().map(|t| escape_pattern(t)).collect::<Vec<_>>().join(".*"));
            return self.t_call("match", vec![text_literal(&pattern)]);
        }

        // `Uppercase<T>` and friends are strings with a shape no runtime check can see.
        if flags.intersects(TypeFlags::StringMapping) {
            return self.t_field("string");
        }

        let Some(symbol) = ty.symbol() else {
            let text = self.cx.checker().type_to_string_exported(ty);
            return Err(self.fail(format!("An unknown type was encountered with no symbol: {text}")));
        };

        let is = |cx: &mut Cx<'_>, name: &str| global_type(cx, name) == Some(symbol);
        if is(self.cx, "Map") || is(self.cx, "ReadonlyMap") || is(self.cx, "WeakMap") {
            let arguments = self.cx.checker().get_type_arguments(ty);
            let key = match arguments.first() {
                Some(&key) => self.build_guard(key)?,
                None => self.t_field("any")?,
            };
            let value = match arguments.get(1) {
                Some(&value) => self.build_guard(value)?,
                None => self.t_field("any")?,
            };
            return self.t_call("map", vec![key, value]);
        }

        if is(self.cx, "Set") || is(self.cx, "ReadonlySet") {
            let value = match self.cx.checker().get_type_arguments(ty).first() {
                Some(&value) => self.build_guard(value)?,
                None => self.t_field("any")?,
            };
            return self.t_call("set", vec![value]);
        }

        if is(self.cx, "Promise") {
            // The TS writes the global `Promise.is`, which roblox-ts compiles to the runtime library's `TS.Promise`.
            // Plugins have no runtime-library primitive yet (see the module docs): the guard library is imported
            // first, which loads the runtime library as every `TS.import` does.
            self.t()?;
            return Ok(create::property(create::property(create::id("TS"), "Promise"), "is"));
        }

        let rbx_symbols = self.rbx_symbols();
        for (name, guard_symbol) in RBX_TYPES.iter().zip(rbx_symbols) {
            if guard_symbol.is_none() && symbol.name() == *name {
                return Err(self.fail(format!("Could not find symbol for {name}")));
            }
            if guard_symbol == Some(symbol) {
                if RBX_TYPES_NEW.contains(name) {
                    return self.t_call("typeof", vec![text_literal(name)]);
                }
                return self.t_field(name);
            }
        }

        if ty.is_class() {
            return Err(self.fail(format!(
                "Class \"{}\" was encountered. Flamework does not support generating guards for classes.",
                symbol.name()
            )));
        }

        let object = is_object(ty);
        let index_infos = self.cx.checker().get_index_infos_of_type(ty);
        let apparent_count = self.cx.checker().get_apparent_properties(ty).len();
        if object && apparent_count == 0 && index_infos.is_empty() {
            return self.t_field("any");
        }

        if object || is_class_or_interface(ty) {
            let mut guards = Vec::new();
            if apparent_count > 0 {
                let fields = self.build_guards_from_type(ty, true)?;
                guards.push(self.t_call("interface", vec![create::map(fields)])?);
            }
            if let Some(&info) = index_infos.first() {
                if index_infos.len() > 1 {
                    return Err(self.fail("Flamework cannot generate types with multiple index signatures."));
                }
                let key = self.build_guard(info.key_type())?;
                let value = self.build_guard(info.value_type())?;
                guards.push(self.t_call("map", vec![key, value])?);
            }
            return match guards.len() {
                0 => {
                    let text = self.cx.checker().type_to_string_exported(ty);
                    Err(self.fail(format!("An unknown type was encountered: {text}")))
                }
                1 => Ok(guards.pop().expect("one")),
                _ => self.list_like("intersection", guards),
            };
        }

        let text = self.cx.checker().type_to_string_exported(ty);
        Err(self.fail(format!("An unknown type was encountered: {text}")))
    }

    /// An instance type: `t.instanceIsA("<Class>")`, with `t.children({ … })` for the properties the class does not
    /// have (an intersection's, taken for children).
    fn build_instance_guard(&mut self, ty: P<Type>) -> PResult<Expr> {
        let instance_type = match instance_type_of(self.cx, ty) {
            Ok(instance_type) => instance_type,
            Err(at) => {
                let at = at.unwrap_or_else(|| self.cx.file().source.get().as_node());
                return Err(self.cx.error(at, "Intersection between nominal types is forbidden."));
            }
        };
        let mut children = Vec::new();
        for &property in self.cx.checker().get_properties_of_type(ty) {
            let property_type = self.cx.checker().get_type_of_property_of_type(ty, property.name());
            let on_class = self.cx.checker().get_property_of_type(instance_type, property.name()).is_some();
            if let Some(property_type) = property_type
                && !on_class
            {
                // Intersections are taken for children.
                let guard = self.build_guard(property_type)?;
                children.push((text_literal(property.name()), guard));
            }
        }

        let class_name = instance_type.symbol().map_or("", |s| s.name());
        let base = self.t_call("instanceIsA", vec![text_literal(class_name)])?;
        if children.is_empty() {
            return Ok(base);
        }
        let children = self.t_call("children", vec![create::map(children)])?;
        self.list_like("intersection", vec![base, children])
    }

    /// `buildRestTupleGuard`: a tuple with a rest element, `[A, B?, ...C[], D]`: a table whose keys are whole numbers
    /// from 1, the elements before the rest in their places, at least `min_length` of them in all, the ones after
    /// the rest at the end, and every one in between a rest element. The element guards are built once, as the
    /// arguments of a function that returns the check.
    fn build_rest_tuple_guard(
        &mut self,
        leading: &[P<Type>],
        rest: Option<P<Type>>,
        trailing: &[P<Type>],
        min_length: usize,
    ) -> PResult<Expr> {
        let all: Vec<P<Type>> = leading.iter().copied().chain(rest).chain(trailing.iter().copied()).collect();
        let mut checks = Vec::new();
        for &ty in &all {
            checks.push(self.build_guard(ty)?);
        }
        let names: Vec<Expr> = (0..checks.len())
            .map(|i| {
                let base = match i.cmp(&leading.len()) {
                    std::cmp::Ordering::Less => "element",
                    std::cmp::Ordering::Equal => "rest",
                    std::cmp::Ordering::Greater => "last",
                };
                self.names.fresh(base)
            })
            .collect();

        let value = self.names.fresh("value");
        let key = self.names.fresh("key");
        let size = self.names.fresh("size");
        let list = self.names.fresh("list");
        let index = self.names.fresh("index");
        let reject = || create::return_stmt(create::bool(false));
        let fail_unless =
            |condition: Expr| create::if_stmt(create::unary(UnaryOperator::Not, condition), vec![reject()], ElseBody::default());
        let check = |name: &Expr, at: Expr| fail_unless(create::call(name.clone(), vec![create::computed_index(list.clone(), at)]));

        let mut body = vec![fail_unless(type_is(value.clone(), "table"))];

        // The highest key, and every key a whole number from 1: `#` is unreliable around nils.
        body.push(create::local(declared(&size), Some(create::number(0.0))));
        let bad_key = create::binary(
            create::binary(
                create::unary(UnaryOperator::Not, type_is(key.clone(), "number")),
                BinaryOperator::Or,
                create::binary(key.clone(), BinaryOperator::Lt, create::number(1.0)),
            ),
            BinaryOperator::Or,
            create::binary(
                create::binary(key.clone(), BinaryOperator::Mod, create::number(1.0)),
                BinaryOperator::Ne,
                create::number(0.0),
            ),
        );
        body.push(create::for_in(
            vec![declared(&key)],
            create::call(create::id("pairs"), vec![value.clone()]),
            vec![
                create::if_stmt(bad_key, vec![reject()], ElseBody::default()),
                create::if_stmt(
                    create::binary(key.clone(), BinaryOperator::Gt, size.clone()),
                    vec![create::assignment(size.clone(), AssignmentOperator::Assign, key.clone())],
                    ElseBody::default(),
                ),
            ],
        ));

        if min_length > 0 {
            body.push(create::if_stmt(
                create::binary(size.clone(), BinaryOperator::Lt, create::number(min_length as f64)),
                vec![reject()],
                ElseBody::default(),
            ));
        }

        body.push(create::local(declared(&list), Some(value.clone())));

        // The elements before the rest, in their places (`list[position]`, offset by roblox-ts).
        for (position, name) in names.iter().take(leading.len()).enumerate() {
            body.push(check(name, create::number((position + 1) as f64)));
        }

        // The rest, between them and the ones after it: `list[index - 1]` is Luau's `list[index]`.
        let end = if trailing.is_empty() {
            size.clone()
        } else {
            create::binary(size.clone(), BinaryOperator::Sub, create::number(trailing.len() as f64))
        };
        body.push(create::numeric_for(
            declared(&index),
            create::number((leading.len() + 1) as f64),
            end,
            None,
            vec![check(&names[leading.len()], index.clone())],
        ));

        // The ones after the rest, at the end: `list[size - (count - position)]`, offset by one.
        for position in 0..trailing.len() {
            let back = trailing.len() - position - 1;
            let at = if back == 0 {
                size.clone()
            } else {
                create::binary(size.clone(), BinaryOperator::Sub, create::number(back as f64))
            };
            body.push(check(&names[leading.len() + 1 + position], at));
        }

        body.push(create::return_stmt(create::bool(true)));

        let guard = create::function(vec![declared(&value)], false, body);
        let parameters = names.iter().map(declared).collect();
        let outer = create::function(parameters, false, vec![create::return_stmt(guard)]);
        Ok(create::call(create::paren(outer), checks))
    }

    /// `buildUnionGuard`
    fn build_union_guard(&mut self, ty: P<Type>) -> PResult<Expr> {
        if ty == self.cx.checker().get_boolean_type() {
            return self.t_field("boolean");
        }

        let simplified = simplify_union(self.cx, ty);
        let (optional, types) = extract_types(self.cx, &simplified.types);
        let mut guards = Vec::new();
        for member in types {
            guards.push(self.build_guard(member)?);
        }
        for name in &simplified.enums {
            let roblox_enum = create::property(create::id("Enum"), name.as_str());
            guards.push(self.t_call("enum", vec![roblox_enum])?);
        }
        if !simplified.literals.is_empty() {
            guards.push(self.list_like("literal", simplified.literals)?);
        }

        let union = match guards.len() {
            0 => return self.t_field("none"),
            1 => guards.pop().expect("one"),
            _ => self.list_like("union", guards)?,
        };
        if optional { self.t_call("optional", vec![union]) } else { Ok(union) }
    }

    /// `buildIntersectionGuard`
    fn build_intersection_guard(&mut self, ty: P<Type>) -> PResult<Expr> {
        if self.cx.checker().get_index_infos_of_type(ty).len() > 1 {
            return Err(self.fail("Flamework cannot generate intersections with multiple index signatures."));
        }

        // An intersection with a disjoint type (a string, a number, …) is that type: the rest are nominal fields.
        if let Some(&disjoint) = ty.types().iter().find(|t| t.flags().intersects(TypeFlags::DisjointDomains)) {
            return self.build_guard(disjoint);
        }

        // A brand on a buffer (`Serialization.buffer16`) is such a field too: a buffer is no table.
        let buffer = global_type(self.cx, "buffer");
        if let Some(&branded) = ty.types().iter().find(|t| buffer.is_some() && t.symbol() == buffer) {
            return self.build_guard(branded);
        }

        let mut guards = Vec::new();
        for &member in ty.types() {
            guards.push(self.build_guard(member)?);
        }
        self.list_like("intersection", guards)
    }

    /// `buildGuardsFromType`: a guard per property. `interface` leaves out properties of type `unknown`, `never`
    /// or a unique symbol, and keys a property by its table key (`{ 10: V }` by the number), as `t.interface`
    /// needs; otherwise every key is the property's name, as a component's attributes need.
    pub(crate) fn build_guards_from_type(&mut self, ty: P<Type>, interface: bool) -> PResult<Vec<(Expr, Expr)>> {
        let declaration = declaration_of_type(ty);
        if let Some(declaration) = declaration {
            self.tracking.push((declaration, ty));
        }

        let mut guards = Vec::new();
        for &property in self.cx.checker().get_properties_of_type(ty) {
            let property_declaration = property.value_declaration();
            let Some(property_type) = self.cx.checker().get_type_of_property_of_type(ty, property.name()) else {
                return Err(self.fail("Could not find type for field"));
            };
            if interface
                && property_type.flags().intersects(TypeFlags::Unknown | TypeFlags::Never | TypeFlags::UniqueESSymbol)
            {
                continue;
            }
            if let Some(property_declaration) = property_declaration {
                self.tracking.push((property_declaration, property_type));
            }
            let guard = self.build_guard(property_type)?;
            let key = if interface { key_expr(&property_key(self.cx, property)) } else { text_literal(property.name()) };
            guards.push((key, guard));
            if property_declaration.is_some() {
                self.tracking.pop();
            }
        }

        if declaration.is_some() {
            self.tracking.pop();
        }
        Ok(guards)
    }
}

/// `typeIs(value, name)` as roblox-ts compiles it: `type(value) == "name"`.
fn type_is(value: Expr, name: &str) -> Expr {
    create::binary(create::call(create::id("type"), vec![value]), BinaryOperator::Eq, text_literal(name))
}

/// `(() => { … })()`: `(function() … end)()`.
fn called_once(body: Vec<Stmt>) -> Expr {
    create::call(create::paren(create::function(Vec::new(), false, body)), Vec::new())
}

/// The TS template-literal escape: `%` before each of `%^$().[]*+-?`.
fn escape_pattern(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "%^$().[]*+-?".contains(c) {
            out.push('%');
        }
        out.push(c);
    }
    out
}

/// `getTypesRequiringDedupHeuristic`: the object and union types appearing at least `limit` times within `ty`, worth
/// declaring once. None for a type nested over [`TYPE_NESTING_LIMIT`] types deep (its guard is then refused, saying
/// where).
fn types_requiring_dedup(cx: &mut Cx<'_>, ty: P<Type>, limit: u32) -> HashSet<P<Type>> {
    struct Walk {
        order: Vec<P<Type>>,
        counts: HashMap<P<Type>, i64>,
        visiting: HashSet<P<Type>>,
    }
    struct NestingLimitReached;

    fn recurse(cx: &mut Cx<'_>, walk: &mut Walk, ty: P<Type>, modifier: i64) -> Result<(), NestingLimitReached> {
        if ty.flags().intersects(TypeFlags::Object | TypeFlags::UnionOrIntersection) {
            match walk.counts.get_mut(&ty) {
                Some(count) => *count += modifier,
                None => {
                    walk.order.push(ty);
                    walk.counts.insert(ty, modifier);
                }
            }
        }
        if walk.visiting.contains(&ty) {
            return Ok(());
        }
        // A type that keeps making new types as it nests never comes back to one being walked: stop altogether.
        if walk.visiting.len() >= TYPE_NESTING_LIMIT {
            return Err(NestingLimitReached);
        }
        walk.visiting.insert(ty);
        let result = recurse_children(cx, walk, ty, modifier);
        walk.visiting.remove(&ty);
        result
    }

    fn recurse_children(cx: &mut Cx<'_>, walk: &mut Walk, ty: P<Type>, modifier: i64) -> Result<(), NestingLimitReached> {
        if ty.flags().intersects(TypeFlags::UnionOrIntersection) {
            for &member in ty.types() {
                recurse(cx, walk, member, modifier)?;
            }
        } else if ty.flags().intersects(TypeFlags::Object) && !is_instance_type(cx, ty) {
            for &property in cx.checker().get_properties_of_type(ty) {
                let Some(property_type) = cx.checker().get_type_of_property_of_type(ty, property.name()) else {
                    continue;
                };
                recurse(cx, walk, property_type, modifier)?;
            }
            for &info in cx.checker().get_index_infos_of_type(ty) {
                recurse(cx, walk, info.key_type(), modifier)?;
                recurse(cx, walk, info.value_type(), modifier)?;
            }
        }
        Ok(())
    }

    let mut walk = Walk { order: Vec::new(), counts: HashMap::new(), visiting: HashSet::new() };
    let mut requires = HashSet::new();
    let outcome = (|| {
        recurse(cx, &mut walk, ty, 1)?;
        // In first-seen order, reading each count as it is now: deduplicating a parent removes `count - 1` of each
        // of its children from the emit, which the children seen after it see.
        let mut i = 0;
        while i < walk.order.len() {
            let ty = walk.order[i];
            let count = walk.counts[&ty];
            if count >= i64::from(limit) {
                requires.insert(ty);
                recurse(cx, &mut walk, ty, -(count - 1))?;
            }
            i += 1;
        }
        Ok(())
    })();
    match outcome {
        Ok(()) => requires,
        Err(NestingLimitReached) => HashSet::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parts_are_escaped_as_lua_patterns() {
        assert_eq!(escape_pattern("a.b(c)%[x]-y?"), "a%.b%(c%)%%%[x%]%-y%?");
        assert_eq!(escape_pattern("-id"), "%-id");
    }

    #[test]
    fn recursion_limit_matches_the_ts() {
        assert_eq!(GUARD_RECURSION_LIMIT, 1000);
        assert_eq!(TYPE_NESTING_LIMIT, 500);
        assert_eq!(GUARD_LOCALS, 150);
    }
}
