//! rbxts-rs: TypeScript 5's TS2769 ("No overload matches this call.") when two or three overloads were tried.
//!
//! TypeScript 5's `reportCallResolutionErrors` elaborates the error of each candidate it tried when there are two or
//! three (`Overload 1 of 2, '<signature>', gave the following error.`, each with its own chain, the related
//! information of all of them on the error), and only the last one's from four on. TypeScript 7
//! (`report_call_resolution_errors`) always elaborates the last one only (`The last overload gave the following
//! error.`, with related information `The last overload is declared here.`). Overload resolution itself is the same in
//! both (and so is context sensitivity, after `mark_ts5_context_sensitive_functions`), but TypeScript 7 drops the
//! candidates it tried once it has reported. So for each such TS2769, the call's resolution is asked again of the
//! checker that checked the file (its candidates, then `choose_overload` over the subtype and the assignable
//! relations, as `resolve_call` asks it), and the error is made again as TypeScript 5 makes it from the candidates
//! that failed on an argument. Calls, `new` expressions, tagged templates, decorators (under their head message,
//! TS1238 and the like) and JSX elements, each with the signatures its `resolve_*` gives `resolve_call`; a call this
//! cannot ask again (one that resolves now, which it never should) keeps TypeScript 7's text.

use tsrs_ast::{Diagnostic, DiagnosticExt, Kind, Node, SourceFile, new_diagnostic, new_diagnostic_chain};
use tsrs_checker::{CallState, CheckMode, Checker, SignatureFlags, SignatureKind, TypeFormatFlags};
use tsrs_core::{P, TextRange};
use tsrs_diagnostics as messages;

/// `Diagnostics.No_overload_matches_this_call.code`.
const NO_OVERLOAD_MATCHES: i32 = 2769;

/// `diagnostics`, with each TS2769 of two or three tried overloads in TypeScript 5's form.
pub(crate) fn ts5_overload_errors(
    checker: &mut Checker,
    file: P<SourceFile>,
    diagnostics: Vec<P<Diagnostic>>,
) -> Vec<P<Diagnostic>> {
    if !diagnostics.iter().any(|d| is_last_overload_error(*d)) {
        return diagnostics;
    }
    diagnostics
        .into_iter()
        .map(|d| {
            if is_last_overload_error(d) && d.file() == Some(file) { rewrite(checker, file, d).unwrap_or(d) } else { d }
        })
        .collect()
}

/// TypeScript 7's TS2769 of several candidates: "The last overload gave the following error." under it, the TS2769
/// itself under a head message for a decorator.
fn is_last_overload_error(d: P<Diagnostic>) -> bool {
    let last_overload = |d: P<Diagnostic>| {
        d.code() == NO_OVERLOAD_MATCHES
            && d.message_chain()
                .first()
                .is_some_and(|c| c.code() == messages::The_last_overload_gave_the_following_error.code())
    };
    last_overload(d) || d.message_chain().first().is_some_and(|&c| last_overload(c))
}

/// The nodes `resolve_call` resolves.
fn is_call_like(node: P<Node>) -> bool {
    matches!(
        node.kind(),
        Kind::CallExpression
            | Kind::NewExpression
            | Kind::TaggedTemplateExpression
            | Kind::Decorator
            | Kind::JsxOpeningElement
            | Kind::JsxSelfClosingElement
    )
}

/// `getDiagnosticHeadMessageForDecoratorResolution`.
fn decorator_head_message(decorator: P<Node>) -> Option<&'static messages::Message> {
    Some(match decorator.parent()?.kind() {
        Kind::ClassDeclaration | Kind::ClassExpression => {
            &messages::Unable_to_resolve_signature_of_class_decorator_when_called_as_an_expression
        }
        Kind::Parameter => &messages::Unable_to_resolve_signature_of_parameter_decorator_when_called_as_an_expression,
        Kind::PropertyDeclaration => {
            &messages::Unable_to_resolve_signature_of_property_decorator_when_called_as_an_expression
        }
        Kind::MethodDeclaration | Kind::GetAccessor | Kind::SetAccessor => {
            &messages::Unable_to_resolve_signature_of_method_decorator_when_called_as_an_expression
        }
        _ => return None,
    })
}

/// The calls whose argument list holds `pos`, innermost first.
fn calls_at(node: P<Node>, pos: u32, out: &mut Vec<P<Node>>) {
    node.for_each_child(&mut |child| {
        let (start, end) = (child.pos().expect_source("a parsed node"), child.end().expect_source("a parsed node"));
        if start <= pos && pos < end {
            if is_call_like(child) {
                out.push(child);
            }
            calls_at(child, pos, out);
        }
        false
    });
}

fn rewrite(checker: &mut Checker, file: P<SourceFile>, d: P<Diagnostic>) -> Option<P<Diagnostic>> {
    let pos = d.pos()?;
    let mut calls = Vec::new();
    calls_at(file.as_node(), pos, &mut calls);
    // innermost first: the call whose last candidate's error is this one
    calls.into_iter().rev().find_map(|call| ts5_error(checker, file, call, d))
}

/// The candidate signatures `resolve_call` got for `call`, and its call chain flags (`resolve_call_expression`,
/// `resolve_new_expression`, `resolve_tagged_template_expression`, `resolve_decorator`,
/// `resolve_jsx_opening_like_element`).
fn signatures_of(checker: &mut Checker, call: P<Node>) -> Option<(Vec<P<tsrs_checker::Signature>>, SignatureFlags)> {
    match call.kind() {
        Kind::TaggedTemplateExpression => {
            let tag_type = checker.check_expression(call.as_tagged_template_expression().tag);
            let apparent = checker.get_apparent_type(tag_type);
            return Some((checker.get_signatures_of_type(apparent, SignatureKind::Call).to_vec(), SignatureFlags::None));
        }
        Kind::JsxOpeningElement | Kind::JsxSelfClosingElement => {
            let tag_name = call.tag_name();
            // an intrinsic element has one made-up signature (`isJsxIntrinsicTagName`)
            if (tag_name.kind() == Kind::Identifier && tsrs_scanner::is_intrinsic_jsx_name(tag_name.text()))
                || tag_name.kind() == Kind::JsxNamespacedName
            {
                return None;
            }
            let element_type = checker.check_expression(tag_name);
            return Some((checker.get_uninstantiated_jsx_signatures_of_type(element_type, call), SignatureFlags::None));
        }
        _ => {}
    }
    let expression = call.expression()?;
    match call.kind() {
        Kind::Decorator => {
            let func_type = checker.check_expression(expression);
            let apparent = checker.get_apparent_type(func_type);
            Some((checker.get_signatures_of_type(apparent, SignatureKind::Call).to_vec(), SignatureFlags::None))
        }
        Kind::CallExpression => {
            if matches!(expression.kind(), Kind::SuperKeyword | Kind::ImportKeyword) {
                return None;
            }
            let mut func_type = checker.check_expression(expression);
            let mut flags = SignatureFlags::None;
            if call.flags().intersects(tsrs_ast::NodeFlags::OptionalChain) {
                let non_optional = checker.get_optional_expression_type(func_type, expression);
                if non_optional != func_type {
                    flags = if tsrs_ast::is_outermost_optional_chain(call) {
                        SignatureFlags::IsOuterCallChain
                    } else {
                        SignatureFlags::IsInnerCallChain
                    };
                }
                func_type = non_optional;
            }
            let func_type = checker.get_non_nullable_type(func_type);
            let apparent = checker.get_apparent_type(func_type);
            Some((checker.get_signatures_of_type(apparent, SignatureKind::Call).to_vec(), flags))
        }
        Kind::NewExpression => {
            let expression_type = checker.check_expression(expression);
            let expression_type = checker.get_non_nullable_type(expression_type);
            let apparent = checker.get_apparent_type(expression_type);
            Some((checker.get_signatures_of_type(apparent, SignatureKind::Construct).to_vec(), SignatureFlags::None))
        }
        _ => None,
    }
}

/// TypeScript 5's TS2769 for `call`, when its resolution fails with two or three candidates that failed on an
/// argument and TypeScript 7's error `d` is the last one's.
fn ts5_error(checker: &mut Checker, file: P<SourceFile>, call: P<Node>, d: P<Diagnostic>) -> Option<P<Diagnostic>> {
    let (signatures, flags) = signatures_of(checker, call)?;
    // resolve_call's state, as it makes it
    let mut s = CallState { node: Some(call), ..CallState::default() };
    if call.kind() != Kind::Decorator {
        s.type_arguments = call.type_arguments().to_vec();
    }
    s.candidates = checker.reorder_candidates(&signatures, flags);
    if s.candidates.is_empty() {
        return None;
    }
    s.args = checker.get_effective_call_arguments(call);
    s.is_single_non_generic_candidate = s.candidates.len() == 1 && s.candidates[0].type_parameters().is_empty();
    let args = s.args.clone();
    let context_sensitive = !s.is_single_non_generic_candidate && args.iter().any(|&a| checker.is_context_sensitive(a));
    s.arg_check_mode = if context_sensitive { CheckMode::SkipContextSensitive } else { CheckMode::Normal };
    s.recursive_resolution = checker.call_resolution_stack.contains(&call);
    checker.call_resolution_stack.push(call);
    let mut result = None;
    if s.candidates.len() > 1 {
        let subtype = checker.subtype_relation;
        result = checker.choose_overload(&mut s, subtype);
    }
    if result.is_none() {
        let assignable = checker.assignable_relation;
        result = checker.choose_overload(&mut s, assignable);
    }
    checker.call_resolution_stack.pop();
    let failed = s.candidates_for_argument_error.clone();
    if result.is_some() || !(2..=3).contains(&failed.len()) {
        return None;
    }
    let assignable = checker.assignable_relation;
    // TypeScript 7's error is the last candidate's: the same place
    let mut last = Vec::new();
    checker.is_signature_applicable(
        call,
        &args,
        failed[failed.len() - 1],
        assignable,
        CheckMode::Normal,
        true,
        Some(&mut last),
    );
    if !last.iter().any(|l| l.loc() == d.loc()) {
        return None;
    }

    // reportCallResolutionErrors, two or three candidates
    let count = s.candidates.len();
    let mut all: Vec<Vec<P<Diagnostic>>> = Vec::new();
    let (mut max, mut min, mut min_index) = (0, usize::MAX, 0);
    for (i, &candidate) in failed.iter().enumerate() {
        let mut diags = Vec::new();
        checker.is_signature_applicable(call, &args, candidate, assignable, CheckMode::Normal, true, Some(&mut diags));
        // `signatureToString` without a kind: a construct signature is written as a call too (no `new`)
        let signature =
            checker.signature_to_string_ex(candidate, None, TypeFormatFlags::WriteCallStyleSignature, None);
        let index = i + 1;
        let diags: Vec<P<Diagnostic>> = diags
            .into_iter()
            .map(|e| {
                new_diagnostic_chain(
                    e,
                    &messages::Overload_0_of_1_2_gave_the_following_error,
                    &[&index, &count, &signature],
                )
            })
            .collect();
        if !diags.is_empty() {
            if diags.len() <= min {
                min = diags.len();
                min_index = i;
            }
            max = max.max(diags.len());
        }
        all.push(diags);
    }
    let diags: Vec<P<Diagnostic>> =
        if max > 1 { all.swap_remove(min_index) } else { all.into_iter().flatten().collect() };
    if diags.is_empty() {
        return None;
    }
    let related: Vec<P<Diagnostic>> = diags.iter().flat_map(|e| e.related_information().iter().copied()).collect();
    let first = diags[0];
    let loc = if diags.iter().all(|e| e.loc() == first.loc() && e.file() == first.file()) {
        first.loc()
    } else {
        let error_node = error_node_for_call_node(call);
        Some(tsrs_scanner::get_error_range_for_node(file, error_node))
    };
    let file_of = if loc == first.loc() { first.file() } else { Some(file) };
    let loc = loc.unwrap_or(TextRange::new(0, 0));
    let mut error = new_diagnostic(file_of, loc, &messages::No_overload_matches_this_call, &[]).set_message_chain(&diags);
    if call.kind() == Kind::Decorator
        && let Some(head) = decorator_head_message(call)
    {
        error = new_diagnostic(file_of, loc, head, &[]).set_message_chain(&[error]);
    }
    let error = error.set_related_info(&related);
    checker.add_implementation_success_elaboration(&s, failed[0], error);
    Some(error)
}

/// TypeScript 5's `getErrorNodeForCallNode`: a call's or `new` expression's callee (its name for a property access),
/// a tagged template's tag (the same), a JSX element's tag name.
fn error_node_for_call_node(call: P<Node>) -> P<Node> {
    let callee_or_name = |callee: P<Node>| {
        if callee.kind() == Kind::PropertyAccessExpression { callee.name().expect("an access has a name") } else { callee }
    };
    match call.kind() {
        Kind::CallExpression | Kind::NewExpression => callee_or_name(call.expression().expect("a call has a callee")),
        Kind::TaggedTemplateExpression => callee_or_name(call.as_tagged_template_expression().tag),
        Kind::JsxOpeningElement | Kind::JsxSelfClosingElement => call.tag_name(),
        _ => call,
    }
}
