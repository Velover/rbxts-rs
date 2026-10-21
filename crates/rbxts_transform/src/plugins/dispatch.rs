//! The claim points and broadcast hooks the transform calls (DESIGN §9b "Plugin dispatch", §11.5). Each returns at
//! once when the build has no plugins (`state.plugins` is `None`) or no plugin asked for that kind of node.

use rustc_hash::FxHashSet;

use rbxts_luau::{AnyIdentifier, Expr, Stmt, create};
use rbxts_plugin::{CallKind, CallSite, Claim, ClassFilter, ClassSite, SymbolRole, Tag};
use tsrs_ast::{Kind, ModifierFlags, Node, SourceFile, Symbol, SymbolFlags};
use tsrs_core::P;

use super::HookKind;
use super::backend::{CallContext, CallShape, access_call_kind, run_classify, run_hook, run_hook_framed};
// (run_hook: the broadcast hooks; run_hook_framed: the claim hooks, whose frame decides a decline)
use super::file_state::{EffectMark, HookFrame, ImportOverrides, PluginImport, RootRange, declared_locals};
use super::host::PluginHost;
use crate::classes::prereqs::{Prereqs, temp_id};
use crate::classes::transform_state::TransformState;
use crate::nodes::statements::transform_import_declaration::transform_import_declaration;
use crate::nodes::statements::transform_import_equals_declaration::transform_import_equals_declaration;
use crate::nodes::transform_optional_chain::{ChainItem, ChainItemKind, transform_chain_item};
use crate::util::create_import_expression::{create_import_expression_of_file, get_source_file_from_module_specifier};
use crate::util::symbols::is_used_as_statement;
use crate::util::traversal::{has_syntactic_modifier, skip_downwards};

// --- imports seen by the compiler ---

/// The compiler compiled a reference to `symbol` at `node` (`transformIdentifier`): when `symbol` is an import
/// alias, a plugin's elision request cannot drop that import.
#[inline]
pub fn note_alias_reference(state: &mut TransformState<'_>, node: P<Node>, symbol: P<Symbol>) {
    if state.plugins.is_none() || !symbol.flags().intersects(SymbolFlags::Alias) {
        return;
    }
    // the alias's own name in its declaration is not a reference
    if node.parent().is_some_and(|p| {
        matches!(
            p.kind(),
            Kind::ImportSpecifier | Kind::ImportClause | Kind::NamespaceImport | Kind::ImportEqualsDeclaration
        )
    }) {
        return;
    }
    state.plugin_file.compiled_aliases.extend(symbol.declarations().iter().copied());
}

/// The elision decision for an import alias while its declaration is transformed again at the end of the file:
/// `Some(false)` elided, `Some(true)` kept, `None` as the checker says.
#[inline]
pub fn import_override(state: &TransformState<'_>, alias_declaration: P<Node>) -> Option<bool> {
    let overrides = state.plugin_file.import_overrides.as_ref()?;
    if overrides.elided.contains(&alias_declaration) {
        Some(false)
    } else if overrides.forced.contains(&alias_declaration) {
        Some(true)
    } else {
        None
    }
}

// --- calls ---

/// `get_property_of_type(get_apparent_type(non_nullable(t)), marker)` is `Some`.
fn has_marker(state: &mut TransformState<'_>, t: tsrs_core::P<tsrs_checker::Type>, marker: &str) -> bool {
    let t = state.checker.get_non_nullable_type(t);
    let t = state.checker.get_apparent_type(t);
    state.checker.get_property_of_type_exported(t, marker).is_some()
}

/// The symbol at a callee node, aliases skipped.
fn callee_symbol(state: &mut TransformState<'_>, node: Option<P<Node>>) -> Option<P<Symbol>> {
    let symbol = state.checker.get_symbol_at_location_exported(node?)?;
    Some(state.checker.skip_alias(symbol))
}

/// `classify_symbol`, memoised per checker and `(symbol, role)`.
fn classify(
    state: &mut TransformState<'_>,
    host: &PluginHost,
    plugin: usize,
    symbol: P<Symbol>,
    role: SymbolRole,
    anchor: P<Node>,
) -> Option<Tag> {
    if let Some(&tag) = host.plugins[plugin].memo.borrow().get(&(symbol, role)) {
        return tag;
    }
    let tag = run_classify(state, plugin, anchor, |instance, cx| instance.classify_symbol(cx, symbol, role));
    host.plugins[plugin].memo.borrow_mut().insert((symbol, role), tag);
    tag
}

/// The plugins whose call interests match, with their tags, in registration order: the symbol classification,
/// then receiver interests, then callee markers.
fn call_candidates(
    state: &mut TransformState<'_>,
    host: &PluginHost,
    site: &CallSite,
    method: Option<&str>,
) -> Vec<(usize, Tag)> {
    let mut out: Vec<(usize, Tag)> = Vec::new();
    let push = |out: &mut Vec<(usize, Tag)>, candidate: (usize, Tag)| {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    };
    let wants_method = method.is_some_and(|m| host.receiver_methods.contains(m)) && site.receiver.is_some();
    let mut receiver_type = None;
    let mut callee_type = None;
    for (plugin, entry) in host.plugins.iter().enumerate() {
        if !state.plugin_file.begun[plugin] {
            continue;
        }
        if entry.calls_by_symbol
            && let Some(symbol) = site.symbol
            && let Some(tag) = classify(state, host, plugin, symbol, SymbolRole::Callee, site.node)
        {
            push(&mut out, (plugin, tag));
        }
        if wants_method {
            let method = method.expect("checked above");
            for interest in &entry.receivers {
                if interest.methods.contains(&method) {
                    let receiver = site.receiver.expect("checked above");
                    let t = *receiver_type.get_or_insert_with(|| state.get_type(receiver));
                    if has_marker(state, t, interest.marker) {
                        push(&mut out, (plugin, interest.tag));
                    }
                }
            }
        }
        for &(marker, tag) in &entry.callee_markers {
            let t = *callee_type.get_or_insert_with(|| state.get_type(site.callee));
            if has_marker(state, t, marker) {
                push(&mut out, (plugin, tag));
            }
        }
    }
    out
}

/// Offers a call to the candidates in order; the first `Take` wins. A hook's `Err` claims the call as `nil` (its
/// error is recorded). `None` when every plugin declined.
fn offer_call(
    state: &mut TransformState<'_>,
    host: &PluginHost,
    prereqs: &mut Prereqs,
    context: &mut CallContext,
    candidates: &[(usize, Tag)],
) -> Option<Expr> {
    let site = context.site;
    // the node is offered once: a dispatch of it while this offer runs lowers it without asking again
    if !state.plugin_file.in_offer.insert(site.node) {
        return None;
    }
    let value = offer_call_inner(state, host, prereqs, context, candidates);
    state.plugin_file.in_offer.remove(&site.node);
    value
}

fn offer_call_inner(
    state: &mut TransformState<'_>,
    host: &PluginHost,
    prereqs: &mut Prereqs,
    context: &mut CallContext,
    candidates: &[(usize, Tag)],
) -> Option<Expr> {
    let start = state.plugin_file.effect_mark(prereqs.statements.len());
    let site = context.site;
    for (position, &(plugin, tag)) in candidates.iter().enumerate() {
        let (result, frame) =
            run_hook_framed(state, Some(prereqs), Some(context), plugin, HookKind::Call, site.node, |instance, cx| {
                instance.call(cx, &site, tag)
            });
        match result {
            Ok(Claim::Take(value)) => {
                if host.claims_all {
                    report_double_claims(state, host, context, plugin, &candidates[position + 1..]);
                }
                return Some(value);
            }
            Ok(Claim::Decline) => {
                if check_decline(state, host, prereqs, &mut *context, plugin, start, frame) {
                    // already an error: no other plugin, and not roblox-ts, gets the node (nothing runs twice)
                    return Some(create::nil());
                }
            }
            Err(()) => return Some(create::nil()),
        }
    }
    None
}

/// What makes a decline wrong, from the declining hook's frame and the effect marks: the hook lowered the node or
/// left something of its own (`frame.lowered`), something grew in the offer's scope or the file (its own effects
/// or a nested hook's), or a hook ran inside it (it would run again when the node is lowered as usual). What a
/// reused receiver left is allowed (`allowed`, `allowed_nested`).
fn decline_problem(
    frame: &HookFrame,
    now: &EffectMark,
    allowed: &EffectMark,
    allowed_nested: u32,
) -> Option<&'static str> {
    frame.lowered.or_else(|| now.growth_since(allowed)).or_else(|| {
        (frame.nested_hooks > allowed_nested)
            .then_some("a node inside it was offered to a plugin hook (it would be offered again)")
    })
}

/// A declined call must leave nothing behind (its own effects or those of hooks nested in it, through
/// `compile_expr` of an argument) but a receiver the next plugin or roblox-ts reuses. Reports and returns `true`
/// when it did.
fn check_decline(
    state: &mut TransformState<'_>,
    host: &PluginHost,
    prereqs: &Prereqs,
    context: &mut CallContext,
    plugin: usize,
    start: EffectMark,
    frame: HookFrame,
) -> bool {
    // a receiver compiled first into the offer's scope (compile_receiver) stays for the next plugin, or roblox-ts
    let (allowed, allowed_nested) = match context.base_marks {
        Some((base_start, base_end)) if base_start == start => (base_end, frame.receiver_nested_hooks),
        _ => (start, 0),
    };
    let now = state.plugin_file.effect_mark(prereqs.statements.len());
    let problem = decline_problem(&frame, &now, &allowed, allowed_nested);
    if let Some(problem) = problem {
        let message = format!("plugin `{}` declined a call after {problem}", host.name(plugin));
        state.internal_error(context.site.node, &message);
    }
    problem.is_some()
}

/// `RBXTS_PLUGIN_CLAIMS=all`: asks the remaining candidates too, in a scratch scope, and reports each that also
/// takes the call. A debug switch must not change the output: a candidate whose hook changed any compiler state
/// before declining (it compiled, placed, imported, named or reported anything) fails the build instead, so a
/// build with the switch either matches the build without it or writes nothing. (The hook's own instance state is
/// out of reach: hooks should decide from the AST and types before doing anything, as the decline rule asks.)
fn report_double_claims(
    state: &mut TransformState<'_>,
    host: &PluginHost,
    context: &CallContext,
    winner: usize,
    rest: &[(usize, Tag)],
) {
    let site = context.site;
    for &(plugin, tag) in rest {
        if plugin == winner {
            continue;
        }
        let mut scratch = Prereqs::new();
        let mut copy = context.clone();
        let (result, frame) = run_hook_framed(
            state,
            Some(&mut scratch),
            Some(&mut copy),
            plugin,
            HookKind::Call,
            site.node,
            |instance, cx| instance.call(cx, &site, tag),
        );
        let message = match result {
            Ok(Claim::Take(_)) => Some(format!(
                "plugins `{}` and `{}` both claimed this call (RBXTS_PLUGIN_CLAIMS=all)",
                host.name(winner),
                host.name(plugin)
            )),
            _ if frame.touched || frame.nested_hooks > 0 => Some(format!(
                "plugin `{}` changed compiler state when asked about a call plugin `{}` claimed \
                 (RBXTS_PLUGIN_CLAIMS=all fails such a build rather than change its output)",
                host.name(plugin),
                host.name(winner)
            )),
            _ => None,
        };
        if let Some(message) = message {
            state.internal_error(site.node, &message);
        }
    }
}

/// The string text of an element access key, for receiver interests and symbols.
fn literal_key(argument: P<Node>) -> Option<&'static str> {
    matches!(argument.kind(), Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral).then(|| argument.text())
}

/// The call site of a chain call: the site (kind still `Call` for property calls), its shape, the method name for
/// receiver interests, and why it is not offered yet when it is one of the S2 shapes. `None` for links that are not
/// calls, and for `import()`.
fn chain_site(
    state: &mut TransformState<'_>,
    host: &PluginHost,
    item: &ChainItem,
) -> Option<(CallContext, Option<&'static str>, Option<&'static str>)> {
    let mut unsupported = None;
    let node = item.node;
    let (shape, callee, symbol_node, receiver, method) = match item.kind {
        ChainItemKind::Call => {
            if tsrs_ast::is_import_call(node) {
                return None;
            }
            let callee = node.expression()?;
            let inner = skip_downwards(callee);
            let shape = if inner.kind() == Kind::SuperKeyword { CallShape::Super } else { CallShape::Call };
            (shape, callee, Some(inner), None, None)
        }
        ChainItemKind::PropertyCall { expression: access, name, call_optional } => {
            // rbxts-rs: S2 — optional method calls (`o.m?.()`) and `super.m()` are not offered yet; a plugin
            // whose interest matches one gets an internal error (`report_unoffered`)
            unsupported = unoffered_reason(access, call_optional);
            let receiver = access.as_property_access_expression().expression;
            (CallShape::Property { access, name }, access, Some(access), Some(receiver), Some(name))
        }
        ChainItemKind::ElementCall { expression: access, argument_expression, call_optional } => {
            unsupported = unoffered_reason(access, call_optional);
            let receiver = access.as_element_access_expression().expression;
            let key = literal_key(argument_expression);
            let shape = CallShape::Element { access, argument: argument_expression };
            (shape, access, key.map(|_| argument_expression), Some(receiver), key)
        }
        ChainItemKind::PropertyAccess { .. } | ChainItemKind::ElementAccess { .. } => return None,
    };
    let symbol = if host.any_symbol { callee_symbol(state, symbol_node) } else { None };
    let kind = if matches!(shape, CallShape::Super) { CallKind::SuperCall } else { CallKind::Call };
    let site = CallSite {
        node,
        kind,
        callee,
        args: node.arguments(),
        symbol,
        receiver,
        in_optional_chain: tsrs_ast::is_optional_chain(node),
        value_used: !is_used_as_statement(node),
    };
    let mut context = CallContext::new(site, shape, None);
    context.item = Some(*item);
    Some((context, method, unsupported))
}

/// Why a property or element call is not offered yet (S2).
fn unoffered_reason(access: P<Node>, call_optional: bool) -> Option<&'static str> {
    if tsrs_ast::is_super_property(access) {
        Some("`super.m()` calls")
    } else if call_optional {
        Some("optional method calls (`o.m?.()`, `o[k]?.()`)")
    } else {
        None
    }
}

/// A call of a shape S2 offers matched a plugin's interest: an internal error naming each such plugin, so the gap
/// is never silent.
fn report_unoffered(
    state: &mut TransformState<'_>,
    host: &PluginHost,
    context: &CallContext,
    method: Option<&str>,
    reason: &str,
) {
    let candidates = call_candidates(state, host, &context.site, method);
    let mut named = Vec::new();
    for (plugin, _) in candidates {
        if !named.contains(&plugin) {
            named.push(plugin);
            let message = format!(
                "plugin `{}`: this call matches its interest, but {reason} are not offered to plugins until S2",
                host.name(plugin)
            );
            state.internal_error(context.site.node, &message);
        }
    }
}

/// An optional method call (`o.m?.()`), which the chain lowering builds itself: not offered yet, so a plugin
/// whose interest matches it gets an internal error.
#[inline]
pub fn check_unoffered_call(state: &mut TransformState<'_>, item: &ChainItem) {
    if state.plugins.is_some() {
        check_unoffered_call_slow(state, item);
    }
}

#[inline(never)]
fn check_unoffered_call_slow(state: &mut TransformState<'_>, item: &ChainItem) {
    let Some(host) = state.plugins else { return };
    if !host.any_calls {
        return;
    }
    if let Some((context, method, Some(reason))) = chain_site(state, host, item) {
        report_unoffered(state, host, &context, method, reason);
    }
}

/// Fills in the method-call kind of a property or element call, once some plugin wants the call.
fn finish_kind(state: &mut TransformState<'_>, context: &mut CallContext) {
    if let CallShape::Property { access, .. } | CallShape::Element { access, .. } = context.shape {
        context.site.kind = access_call_kind(state, access);
    }
}

/// The first call of a chain whose callee is reached through plain accesses only (`f()`, `ns.f()`, `a.b.m()`): it is
/// offered before anything of the chain is compiled. Returns the chain index to continue from and the value so far
/// (the claimed value, or the call lowered as usual when a plugin compiled its receiver and then declined); `None`
/// to lower the chain as usual (the call is then not offered again).
#[inline]
pub fn claim_chain_start(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    chain: &[ChainItem],
) -> Option<(usize, Expr)> {
    state.plugins?;
    claim_chain_start_slow(state, prereqs, chain)
}

#[inline(never)]
fn claim_chain_start_slow(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    chain: &[ChainItem],
) -> Option<(usize, Expr)> {
    let host = state.plugins?;
    if !host.any_calls {
        return None;
    }
    let k = chain.iter().position(|item| {
        !matches!(item.kind, ChainItemKind::PropertyAccess { .. } | ChainItemKind::ElementAccess { .. })
    })?;
    if chain[..=k].iter().any(|item| item.optional) {
        return None;
    }
    let item = &chain[k];
    if state.plugin_file.in_offer.contains(&item.node) {
        return None;
    }
    if state.plugin_file.offered.contains(&item.node) {
        // `compile_expr` of the claimed call itself: the item lowering skips it
        return None;
    }
    let (mut context, method, unsupported) = chain_site(state, host, item)?;
    if unsupported.is_some() {
        // the chain lowering reports it
        return None;
    }
    let candidates = call_candidates(state, host, &context.site, method);
    // asked now or never: the chain lowering does not ask again
    state.plugin_file.offered.insert(item.node);
    if candidates.is_empty() {
        return None;
    }
    finish_kind(state, &mut context);
    if let Some(value) = offer_call(state, host, prereqs, &mut context, &candidates) {
        state.plugin_file.offered.remove(&item.node);
        return Some((k + 1, value));
    }
    // (an erroneous decline returned nil above, so the node is not lowered or offered again)
    // declined: a `compile_expr` of the call in a hook consumed the mark, which keeps it from being offered again
    state.plugin_file.offered.insert(item.node);
    if context.base_captured {
        // its prereqs were the plugin's (and the decline is an error): lowered again as usual
        return None;
    }
    let base = context.base.take()?;
    Some((k + 1, transform_chain_item(state, prereqs, base, item)))
}

/// A chain call reached by the chain lowering, its receiver (or callee) compiled as `base` and nil-checked inside
/// an optional chain. `Ok` with the claimed value, or `Err(base)` to lower it as usual.
#[inline]
pub fn claim_chain_call(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    item: &ChainItem,
    base: Expr,
) -> Result<Expr, Expr> {
    if state.plugins.is_none() {
        return Err(base);
    }
    claim_chain_call_slow(state, prereqs, item, base)
}

#[inline(never)]
fn claim_chain_call_slow(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    item: &ChainItem,
    base: Expr,
) -> Result<Expr, Expr> {
    let Some(host) = state.plugins else { return Err(base) };
    if !host.any_calls
        || matches!(item.kind, ChainItemKind::PropertyAccess { .. } | ChainItemKind::ElementAccess { .. })
    {
        return Err(base);
    }
    if state.plugin_file.in_offer.contains(&item.node) || state.plugin_file.offered.remove(&item.node) {
        return Err(base);
    }
    let Some((mut context, method, unsupported)) = chain_site(state, host, item) else { return Err(base) };
    if let Some(reason) = unsupported {
        report_unoffered(state, host, &context, method, reason);
        return Err(base);
    }
    let candidates = call_candidates(state, host, &context.site, method);
    if candidates.is_empty() {
        return Err(base);
    }
    finish_kind(state, &mut context);
    context.base = Some(base);
    match offer_call(state, host, prereqs, &mut context, &candidates) {
        Some(value) => Ok(value),
        None => Err(context.base.take().expect("the chain's base")),
    }
}

/// `new C(…)`, before constructor macros.
#[inline]
pub fn claim_new(state: &mut TransformState<'_>, prereqs: &mut Prereqs, node: P<Node>) -> Option<Expr> {
    state.plugins?;
    claim_new_slow(state, prereqs, node)
}

#[inline(never)]
fn claim_new_slow(state: &mut TransformState<'_>, prereqs: &mut Prereqs, node: P<Node>) -> Option<Expr> {
    let host = state.plugins?;
    if !host.any_calls || state.plugin_file.in_offer.contains(&node) || state.plugin_file.offered.remove(&node) {
        return None;
    }
    let callee = node.expression()?;
    let symbol = if host.any_symbol { callee_symbol(state, Some(skip_downwards(callee))) } else { None };
    let site = CallSite {
        node,
        kind: CallKind::New,
        callee,
        args: node.arguments(),
        symbol,
        receiver: None,
        in_optional_chain: false,
        value_used: !is_used_as_statement(node),
    };
    let candidates = call_candidates(state, host, &site, None);
    if candidates.is_empty() {
        return None;
    }
    let mut context = CallContext::new(site, CallShape::New, None);
    offer_call(state, host, prereqs, &mut context, &candidates)
}

/// A decorator (`@D`, `@D(…)`), before its expression is compiled. The claimed value replaces the decorator's
/// expression, which roblox-ts then applies as usual.
#[inline]
pub fn claim_decorator(state: &mut TransformState<'_>, prereqs: &mut Prereqs, decorator: P<Node>) -> Option<Expr> {
    state.plugins?;
    claim_decorator_slow(state, prereqs, decorator)
}

#[inline(never)]
fn claim_decorator_slow(state: &mut TransformState<'_>, prereqs: &mut Prereqs, decorator: P<Node>) -> Option<Expr> {
    let host = state.plugins?;
    if !host.any_calls || state.plugin_file.in_offer.contains(&decorator) {
        return None;
    }
    let expression = decorator.expression()?;
    let inner = skip_downwards(expression);
    let (call, callee, args): (Option<P<Node>>, P<Node>, &'static [P<Node>]) = if inner.kind() == Kind::CallExpression {
        (Some(inner), inner.expression()?, inner.arguments())
    } else {
        (None, expression, &[])
    };
    let symbol = if host.any_symbol { callee_symbol(state, Some(skip_downwards(callee))) } else { None };
    let site = CallSite {
        node: decorator,
        kind: CallKind::Decorator,
        callee,
        args,
        symbol,
        receiver: None,
        in_optional_chain: false,
        value_used: true,
    };
    let candidates = call_candidates(state, host, &site, None);
    if candidates.is_empty() {
        return None;
    }
    // offered as a decorator, not again as the call inside it
    if let Some(call) = call {
        state.plugin_file.offered.insert(call);
    }
    let mut context = CallContext::new(site, CallShape::Decorator { call }, None);
    let value = offer_call(state, host, prereqs, &mut context, &candidates);
    if value.is_some()
        && let Some(call) = call
    {
        state.plugin_file.offered.remove(&call);
    }
    value
}

// --- by-kind fallbacks ---

/// The `expressions(kind)` fallback, before roblox-ts's own lowering of the node.
#[inline]
pub fn claim_expression(state: &mut TransformState<'_>, prereqs: &mut Prereqs, node: P<Node>) -> Option<Expr> {
    state.plugins?;
    claim_expression_slow(state, prereqs, node)
}

#[inline(never)]
fn claim_expression_slow(state: &mut TransformState<'_>, prereqs: &mut Prereqs, node: P<Node>) -> Option<Expr> {
    let host = state.plugins?;
    let list = host.expressions.get(&node.kind())?;
    if !state.plugin_file.in_offer.insert(node) {
        return None;
    }
    let value = claim_expression_offer(state, host, prereqs, node, list);
    state.plugin_file.in_offer.remove(&node);
    value
}

fn claim_expression_offer(
    state: &mut TransformState<'_>,
    host: &PluginHost,
    prereqs: &mut Prereqs,
    node: P<Node>,
    list: &[(usize, Tag)],
) -> Option<Expr> {
    for &(plugin, tag) in list {
        if !state.plugin_file.begun[plugin] {
            continue;
        }
        let start = state.plugin_file.effect_mark(prereqs.statements.len());
        let (result, frame) =
            run_hook_framed(state, Some(prereqs), None, plugin, HookKind::Expression, node, |instance, cx| {
                instance.expression(cx, node, tag)
            });
        match result {
            Ok(Claim::Take(value)) => return Some(value),
            Ok(Claim::Decline) => {
                let now = state.plugin_file.effect_mark(prereqs.statements.len());
                if let Some(what) = decline_problem(&frame, &now, &start, 0) {
                    let message = format!("plugin `{}` declined an expression after {what}", host.name(plugin));
                    state.internal_error(node, &message);
                    // already an error: no other plugin, and not roblox-ts, gets the node
                    return Some(create::nil());
                }
            }
            Err(()) => return Some(create::nil()),
        }
    }
    None
}

/// The `statements(kind)` fallback, before roblox-ts's own lowering of the statement. A claim's prereqs come first.
#[inline]
pub fn claim_statement(state: &mut TransformState<'_>, node: P<Node>) -> Option<Vec<Stmt>> {
    state.plugins?;
    claim_statement_slow(state, node)
}

#[inline(never)]
fn claim_statement_slow(state: &mut TransformState<'_>, node: P<Node>) -> Option<Vec<Stmt>> {
    let host = state.plugins?;
    let list = host.statements.get(&node.kind())?;
    if !state.plugin_file.in_offer.insert(node) {
        return None;
    }
    let value = claim_statement_offer(state, host, node, list);
    state.plugin_file.in_offer.remove(&node);
    value
}

fn claim_statement_offer(
    state: &mut TransformState<'_>,
    host: &PluginHost,
    node: P<Node>,
    list: &[(usize, Tag)],
) -> Option<Vec<Stmt>> {
    for &(plugin, tag) in list {
        if !state.plugin_file.begun[plugin] {
            continue;
        }
        let mut prereqs = Prereqs::new();
        let start = state.plugin_file.effect_mark(0);
        let (result, frame) =
            run_hook_framed(state, Some(&mut prereqs), None, plugin, HookKind::Statement, node, |instance, cx| {
                instance.statement(cx, node, tag)
            });
        match result {
            Ok(Claim::Take(statements)) => {
                let mut out = prereqs.statements;
                out.extend(statements);
                return Some(out);
            }
            Ok(Claim::Decline) => {
                let now = state.plugin_file.effect_mark(prereqs.statements.len());
                if let Some(what) = decline_problem(&frame, &now, &start, 0) {
                    let message = format!("plugin `{}` declined a statement after {what}", host.name(plugin));
                    state.internal_error(node, &message);
                    return Some(Vec::new());
                }
            }
            Err(()) => return Some(Vec::new()),
        }
    }
    None
}

// --- classes ---

/// The class, a member, or a parameter of the constructor or a method has a decorator.
fn is_decorated(node: P<Node>) -> bool {
    !node.decorators().is_empty()
        || node.members().iter().any(|&member| {
            !member.decorators().is_empty()
                || (matches!(member.kind(), Kind::MethodDeclaration | Kind::Constructor)
                    && member.parameters().iter().any(|p| !p.decorators().is_empty()))
        })
}

/// The `class` hook: every plugin whose filter admits the class, in registration order, statements concatenated
/// (each plugin's prereqs before its statements). They go at the static-block position: after the statics, before
/// the decorators.
#[inline]
pub fn class_statements(state: &mut TransformState<'_>, node: P<Node>, luau_name: &AnyIdentifier) -> Vec<Stmt> {
    if state.plugins.is_none() {
        return Vec::new();
    }
    class_statements_slow(state, node, luau_name)
}

#[inline(never)]
fn class_statements_slow(state: &mut TransformState<'_>, node: P<Node>, luau_name: &AnyIdentifier) -> Vec<Stmt> {
    let Some(host) = state.plugins else { return Vec::new() };
    if !host.any_classes {
        return Vec::new();
    }
    if !state.plugin_file.in_offer.insert(node) {
        return Vec::new();
    }
    let decorated = is_decorated(node);
    let site = ClassSite { node, luau_name: luau_name.clone().into(), decorated };
    let mut out = Vec::new();
    for plugin in 0..host.len() {
        let admitted = match host.plugins[plugin].class_filter {
            Some(ClassFilter::All) => true,
            Some(ClassFilter::Decorated) => decorated,
            None => false,
        };
        if !admitted || !state.plugin_file.begun[plugin] {
            continue;
        }
        let mut prereqs = Prereqs::new();
        if let Ok(statements) =
            run_hook(state, Some(&mut prereqs), None, plugin, HookKind::Class, node, |instance, cx| {
                instance.class(cx, &site)
            })
        {
            out.extend(prereqs.statements);
            out.extend(statements);
        }
    }
    state.plugin_file.in_offer.remove(&node);
    out
}

// --- files ---

/// `begin_file` of every plugin, before the file's statements are transformed.
#[inline]
pub fn begin_file(state: &mut TransformState<'_>) {
    if state.plugins.is_none() {
        return;
    }
    begin_file_slow(state)
}

#[inline(never)]
fn begin_file_slow(state: &mut TransformState<'_>) {
    let Some(host) = state.plugins else { return };
    let root = state.source_file.as_node();
    for plugin in 0..host.len() {
        let begun =
            run_hook(state, None, None, plugin, HookKind::BeginFile, root, |instance, cx| instance.begin_file(cx))
                .is_ok();
        state.plugin_file.begun[plugin] = begun;
    }
}

/// `end_file` of every plugin (their parts are kept for `finish`), then what was decided only now: import elision
/// and kept imports, and the placement of plugin imports and header statements.
#[inline]
pub fn end_file(state: &mut TransformState<'_>, statements: &mut Vec<Stmt>) {
    if state.plugins.is_none() {
        return;
    }
    end_file_slow(state, statements)
}

#[inline(never)]
fn end_file_slow(state: &mut TransformState<'_>, statements: &mut Vec<Stmt>) {
    let Some(host) = state.plugins else { return };
    let root = state.source_file.as_node();
    for plugin in 0..host.len() {
        if let Ok(part) =
            run_hook(state, None, None, plugin, HookKind::EndFile, root, |instance, cx| instance.end_file(cx))
        {
            state.plugin_file.parts[plugin] = Some(part);
        }
    }
    // only claim hooks insert before root statements, and they run inside statements; nothing should be left
    if !state.plugin_file.root_inserts.is_empty() {
        statements.append(&mut state.plugin_file.root_inserts);
    }
    apply_import_decisions(state, statements);
    place_imports_and_header(state, statements);
}

/// Where a top-level statement's transform began.
#[derive(Clone, Copy, Debug)]
pub struct RootMark {
    diagnostics_start: usize,
    runtime_lib_uses: u32,
}

/// Starts a statement of the file's top level (`parent` is the source file); `None` otherwise or without plugins.
#[inline]
pub fn root_statement_start(state: &TransformState<'_>, parent: Option<P<Node>>) -> Option<RootMark> {
    state.plugins?;
    if parent?.kind() != Kind::SourceFile {
        return None;
    }
    Some(RootMark { diagnostics_start: state.diagnostics.diagnostics.len(), runtime_lib_uses: state.runtime_lib_uses })
}

/// The statements plugins placed before the current top-level statement.
pub fn root_statement_inserts(state: &mut TransformState<'_>, result: &mut Vec<Stmt>) {
    result.append(&mut state.plugin_file.root_inserts);
}

/// Ends a top-level statement whose own Luau is `result[start..]`.
pub fn root_statement_end(
    state: &mut TransformState<'_>,
    mark: RootMark,
    statement: P<Node>,
    result: &[Stmt],
    start: usize,
) {
    state.plugin_file.main_chunk_locals += result[start..].iter().map(declared_locals).sum::<u32>();
    let range = RootRange {
        node: statement,
        start,
        end: result.len(),
        diagnostics_start: mark.diagnostics_start,
        diagnostics_end: state.diagnostics.diagnostics.len(),
        runtime_lib_uses: state.runtime_lib_uses - mark.runtime_lib_uses,
    };
    state.plugin_file.root_ranges.push(range);
}

/// The alias declarations of an import statement.
fn alias_declarations(statement: P<Node>) -> Vec<P<Node>> {
    match statement.kind() {
        Kind::ImportDeclaration => {
            let Some(clause_node) = statement.as_import_declaration().import_clause else { return Vec::new() };
            let clause = clause_node.as_import_clause();
            let mut out = Vec::new();
            if clause.name.is_some() {
                out.push(clause_node);
            }
            if let Some(bindings) = clause.named_bindings {
                if bindings.kind() == Kind::NamespaceImport {
                    out.push(bindings);
                } else {
                    out.extend(bindings.elements().iter().copied());
                }
            }
            out
        }
        Kind::ImportEqualsDeclaration => vec![statement],
        _ => Vec::new(),
    }
}

/// The local name an alias declaration binds.
fn alias_name(alias: P<Node>) -> Option<&'static str> {
    match alias.kind() {
        Kind::ImportClause => alias.as_import_clause().name.map(|n| n.text()),
        Kind::NamespaceImport => Some(alias.as_namespace_import().name.text()),
        Kind::ImportEqualsDeclaration => Some(alias.as_import_equals_declaration().name.text()),
        _ => alias.name().map(|n| n.text()),
    }
}

/// The local names a file re-exports with `export { a, b as c }` (no module specifier).
fn reexported_names(file: P<SourceFile>) -> FxHashSet<&'static str> {
    let mut names = FxHashSet::default();
    for &statement in file.get().statements.nodes() {
        if statement.kind() != Kind::ExportDeclaration {
            continue;
        }
        let declaration = statement.as_export_declaration();
        if declaration.module_specifier.is_some() {
            continue;
        }
        let Some(clause) = declaration.export_clause else { continue };
        if clause.kind() != Kind::NamedExports {
            continue;
        }
        for &specifier in clause.elements() {
            let export = specifier.as_export_specifier();
            let local = export.property_name.unwrap_or(export.name);
            if local.kind() == Kind::Identifier {
                names.insert(local.text());
            }
        }
    }
    names
}

/// Import elision decided after the whole file is lowered: an alias a plugin asked to elide goes unless the
/// compiler compiled a reference to it, a plugin kept it (`mark_import_used`, a reused import), or the file
/// re-exports it; a kept alias is emitted though the checker saw no value use. The affected import declarations
/// are transformed again, and their Luau and diagnostics replace the first transform's.
fn apply_import_decisions(state: &mut TransformState<'_>, statements: &mut Vec<Stmt>) {
    if state.plugin_file.elision_requests.is_empty() && state.plugin_file.forced_imports.is_empty() {
        return;
    }
    let reexported = reexported_names(state.source_file);
    for i in (0..state.plugin_file.root_ranges.len()).rev() {
        let range = state.plugin_file.root_ranges[i];
        let aliases = alias_declarations(range.node);
        // `export import X = …` re-exports X: never elided
        if aliases.is_empty() || has_syntactic_modifier(range.node, ModifierFlags::Export) {
            continue;
        }
        let file = &state.plugin_file;
        let mut overrides = ImportOverrides::default();
        for alias in aliases {
            if file.forced_imports.contains(&alias) {
                overrides.forced.insert(alias);
            } else if file.elision_requests.contains(&alias)
                && !file.compiled_aliases.contains(&alias)
                && !alias_name(alias).is_some_and(|name| reexported.contains(name))
            {
                overrides.elided.insert(alias);
            }
        }
        if overrides.elided.is_empty() && overrides.forced.is_empty() {
            continue;
        }
        state.plugin_file.import_overrides = Some(overrides);
        let diagnostics_before = state.diagnostics.diagnostics.len();
        // the first transform's uses of `TS` are taken back; the new one counts its own
        state.runtime_lib_uses -= range.runtime_lib_uses;
        let uses_before = state.runtime_lib_uses;
        let replacement = if range.node.kind() == Kind::ImportDeclaration {
            transform_import_declaration(state, range.node)
        } else {
            transform_import_equals_declaration(state, range.node)
        };
        state.plugin_file.import_overrides = None;
        state.plugin_file.root_ranges[i].runtime_lib_uses = state.runtime_lib_uses - uses_before;
        state.uses_runtime_lib = state.runtime_lib_uses > 0;
        let new_diagnostics: Vec<_> = state.diagnostics.diagnostics.drain(diagnostics_before..).collect();
        let diagnostics_delta =
            new_diagnostics.len() as isize - (range.diagnostics_end - range.diagnostics_start) as isize;
        state.diagnostics.diagnostics.splice(range.diagnostics_start..range.diagnostics_end, new_diagnostics);
        let delta = replacement.len() as isize - (range.end - range.start) as isize;
        statements.splice(range.start..range.end, replacement);
        let ranges = &mut state.plugin_file.root_ranges;
        ranges[i].end = (range.end as isize + delta) as usize;
        ranges[i].diagnostics_end = (range.diagnostics_end as isize + diagnostics_delta) as usize;
        for later in &mut ranges[i + 1..] {
            later.start = (later.start as isize + delta) as usize;
            later.end = (later.end as isize + delta) as usize;
            later.diagnostics_start = (later.diagnostics_start as isize + diagnostics_delta) as usize;
            later.diagnostics_end = (later.diagnostics_end as isize + diagnostics_delta) as usize;
        }
    }
}

/// Plugin imports go first, after any `--!` directives (as Flamework's added imports did ahead of the user's);
/// header statements after the imports: after the last import of the file's leading run of imports.
fn place_imports_and_header(state: &mut TransformState<'_>, statements: &mut Vec<Stmt>) {
    let imports: Vec<Stmt> = state.plugin_file.imports.iter_mut().filter_map(|i| i.statement.take()).collect();
    let header = std::mem::take(&mut state.plugin_file.header);
    if imports.is_empty() && header.is_empty() {
        return;
    }
    let directives =
        statements.iter().take_while(|s| matches!(s, Stmt::Comment(text) if text.starts_with('!'))).count();
    let mut after_imports = 0;
    for range in &state.plugin_file.root_ranges {
        match range.node.kind() {
            Kind::ImportDeclaration | Kind::ImportEqualsDeclaration => after_imports = range.end,
            _ if range.start == range.end => {}
            _ => break,
        }
    }
    let header_at = after_imports.max(directives) + imports.len();
    statements.splice(directives..directives, imports);
    statements.splice(header_at..header_at, header);
}

/// Whether the local name of an import alias could be shadowed where a plugin uses it: some identifier of the file
/// with that text resolves to anything but the alias (a parameter, a local, a property), so the name does not mean
/// the import everywhere. Conservative: a reused import must mean the import at every use.
fn alias_shadowed(state: &mut TransformState<'_>, alias: P<Node>) -> bool {
    let Some(alias_symbol) = alias.symbol() else { return true };
    let Some(name) = alias_name(alias) else { return true };
    fn walk(node: P<Node>, name: &str, out: &mut Vec<P<Node>>) {
        if node.kind() == Kind::Identifier && node.text() == name {
            out.push(node);
        }
        node.for_each_child(&mut |child| {
            walk(child, name, out);
            false
        });
    }
    let mut identifiers = Vec::new();
    walk(state.source_file.as_node(), name, &mut identifiers);
    identifiers.into_iter().any(|id| state.checker.get_symbol_at_location_exported(id) != Some(alias_symbol))
}

/// A user import of `export` from `module_file` in this file: the alias declaration and the value to use.
fn find_user_import(
    state: &mut TransformState<'_>,
    module_file: P<SourceFile>,
    export: &str,
) -> Option<(P<Node>, Expr)> {
    for &statement in state.source_file.get().statements.nodes() {
        if statement.kind() != Kind::ImportDeclaration {
            continue;
        }
        let declaration = statement.as_import_declaration();
        let Some(clause_node) = declaration.import_clause else { continue };
        let clause = clause_node.as_import_clause();
        if clause.phase_modifier.get() == Kind::TypeKeyword {
            continue;
        }
        if get_source_file_from_module_specifier(state, declaration.module_specifier) != Some(module_file) {
            continue;
        }
        if export == "default"
            && let Some(name) = clause.name
        {
            return Some((clause_node, create::id(name.text())));
        }
        let Some(bindings) = clause.named_bindings else { continue };
        if bindings.kind() == Kind::NamespaceImport {
            let name = bindings.as_namespace_import().name.text();
            return Some((bindings, create::property(create::id(name), export)));
        }
        for &element in bindings.elements() {
            if element.is_type_only() {
                continue;
            }
            let name = element.name().expect("an import specifier has a name");
            let exported = element.property_name().unwrap_or(name);
            if exported.text() == export {
                return Some((element, create::id(name.text())));
            }
        }
    }
    None
}

/// `import_named`: the value of `export` from `specifier`, imported once per file. A user import of the same
/// export is reused (and kept alive); otherwise a `local <export> = TS.import(…).<export>` goes with the plugin
/// imports at the top of the file, named after the export unless a name in the file could shadow it.
pub(crate) fn import_named(
    state: &mut TransformState<'_>,
    host: &PluginHost,
    anchor: P<Node>,
    specifier: &str,
    export: &str,
) -> Result<Expr, String> {
    let key = (specifier.to_owned(), export.to_owned());
    if let Some(value) = state.plugin_file.import_cache.get(&key) {
        return Ok(value.clone());
    }
    let file = state.source_file;
    let resolved = host
        .resolve_module(specifier, file.get().file_name())
        .ok_or_else(|| format!("import_named: `{specifier}` cannot be resolved from {}", file.get().file_name()))?;
    let module_file = state.program.get_source_file(&resolved.file_name).ok_or_else(|| {
        format!("import_named: `{specifier}` resolves to {}, which is not in the program", resolved.file_name)
    })?;

    let reusable = find_user_import(state, module_file, export).filter(|&(alias, _)| !alias_shadowed(state, alias));
    let value = if let Some((alias, value)) = reusable {
        state.plugin_file.forced_imports.insert(alias);
        value
    } else {
        let usable = rbxts_luau::is_valid_identifier(export)
            && !rbxts_luau::is_reserved_identifier(export)
            && !state.plugin_file.name_taken(file, export);
        let id: AnyIdentifier =
            if usable { create::any_id(export) } else { temp_id(state.plugin_file.seam_hint(file, export)).into() };
        state.plugin_file.import_names.insert(export.to_owned());
        let import = create_import_expression_of_file(state, file, module_file, anchor);
        let statement = create::local(id.clone(), Some(create::computed_index(import, create::string(export))));
        state.plugin_file.main_chunk_locals += 1;
        let value: Expr = id.into();
        state.plugin_file.imports.push(PluginImport {
            specifier: specifier.to_owned(),
            export: export.to_owned(),
            value: value.clone(),
            statement: Some(statement),
        });
        value
    };
    state.plugin_file.import_cache.insert(key, value.clone());
    Ok(value)
}
