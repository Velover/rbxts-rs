//! `util/isMethod.ts`: whether calling a value passes a receiver (`self`), decided by its call signatures.

use tsrs_ast::{Kind, Node, Symbol};
use tsrs_checker::{CheckMode, Relation, Signature, SignatureFlags, SignatureKind, Type, TypeFlags};
use tsrs_core::P;

use crate::classes::transform_state::TransformState;
use crate::shared::diagnostics::errors;
use crate::util::traversal::skip_upwards;
use crate::util::types::{get_type_arguments, walk_types};

fn contains_instantiable_type(t: P<Type>) -> bool {
    if t.flags().intersects(TypeFlags::UnionOrIntersection) {
        return t.types().iter().any(|&t| contains_instantiable_type(t));
    }
    t.flags().intersects(TypeFlags::Instantiable)
}

fn can_change_receiver_convention(state: &mut TransformState<'_>, t: P<Type>) -> bool {
    if !contains_instantiable_type(t) {
        return false;
    }

    // a fixed non-void union member keeps the instantiated union from becoming void
    if t.flags().intersects(TypeFlags::Union)
        && t.types()
            .iter()
            .any(|&m| !m.flags().intersects(TypeFlags::Void | TypeFlags::Never) && !contains_instantiable_type(m))
    {
        return false;
    }

    if t.flags().intersects(TypeFlags::Conditional) {
        let conditional = t.as_conditional_type();
        let root = conditional.root.get().expect("a conditional type has a root");
        let mut check_type = root.check_type.get().expect("a conditional root has a check type");
        let mut extends_type = conditional.extends_type.get().expect("a conditional type has an extends type");
        // singleton tuples suppress distribution without changing the void test
        if check_type.is_tuple_type() && extends_type.is_tuple_type() {
            let check_elements = get_type_arguments(state, check_type);
            let extends_elements = get_type_arguments(state, extends_type);
            if check_elements.len() == 1 && extends_elements.len() == 1 {
                check_type = check_elements[0];
                extends_type = extends_elements[0];
            }
        }

        // a conditional that removes void can still have an unconstrained fallback type
        let node = root.node.get().expect("a conditional root has a node").as_conditional_type_node();
        let true_type = state.checker.get_type_from_type_node_exported(node.true_type);
        let false_type = state.checker.get_type_from_type_node_exported(node.false_type);
        let void_type = state.checker.get_void_type();
        if true_type.flags().intersects(TypeFlags::Never)
            && false_type == check_type
            && state.checker.is_type_assignable_to_exported(void_type, extends_type)
        {
            return false;
        }
    }

    match state.checker.get_base_constraint_of_type_exported(t) {
        None => true,
        Some(constraint) => {
            let void_type = state.checker.get_void_type();
            state.checker.is_type_assignable_to_exported(void_type, constraint)
        }
    }
}

fn is_method_declaration(node: P<Node>) -> bool {
    if matches!(node.kind(), Kind::MethodDeclaration | Kind::MethodSignature) {
        return true;
    }
    // object literal function expressions have an implicit receiver
    node.kind() == Kind::FunctionExpression
        && skip_upwards(node).parent().is_some_and(|p| p.kind() == Kind::PropertyAssignment)
}

/// The `this` parameter TypeScript 5 gives a signature that has none of its own.
// rbxts-rs: TypeScript 5 calls a function expression or object literal method without type parameters and without
// an explicit `this` parameter context sensitive, and so gives it its contextual signature's `this` parameter
// (`assignContextualParameterTypes`). TypeScript 7's `hasContextSensitiveParameters` asks whether such a
// function uses `this`, so one that has only annotated parameters and doesn't use `this` keeps no `this`
// parameter, and a `function () {}` in a `(this: defined) => void` slot would lose its `self`. Emulate TypeScript 5
// in exactly that case; with an unannotated parameter, or `this` used, both agree and the signature has it already.
// The contextual signature is TypeScript 5's for an argument of an overloaded call too
// (`ts5_overload_contextual_signature`). Needs the tsrs fork at adadb25 or later, where
// `Checker::get_contextual_signature` is public.
fn ts5_contextual_this_parameter(state: &mut TransformState<'_>, signature: P<Signature>) -> Option<P<Symbol>> {
    let declaration = signature.declaration()?;
    if !(declaration.kind() == Kind::FunctionExpression || tsrs_ast::is_object_literal_method(declaration))
        || declaration.type_parameter_list().is_some()
        || declaration.parameters().iter().any(|p| p.type_node().is_none())
        || declaration.flags().intersects(tsrs_ast::NodeFlags::ContainsThis)
    {
        return None;
    }
    let contextual = match ts5_overload_contextual_signature(state, declaration) {
        Some(contextual) => contextual,
        None => state.checker.get_contextual_signature(declaration),
    };
    contextual?.this_parameter()
}

/// TypeScript 5's `isContextSensitive`: as TypeScript 7's, but a function expression or object literal method without
/// an explicit `this` parameter is context sensitive whether or not it uses `this` (its implicit `this` is
/// contextually typed), and a `yield` is not looked into.
fn ts5_is_context_sensitive(node: P<Node>) -> bool {
    match node.kind() {
        Kind::FunctionExpression | Kind::ArrowFunction | Kind::MethodDeclaration | Kind::FunctionDeclaration => {
            ts5_has_context_sensitive_parameters(node) || ts5_has_context_sensitive_return_expression(node)
        }
        Kind::ObjectLiteralExpression => node.properties().iter().any(|&p| ts5_is_context_sensitive(p)),
        Kind::ArrayLiteralExpression => node.elements().iter().any(|&e| ts5_is_context_sensitive(e)),
        Kind::ConditionalExpression => {
            let conditional = node.as_conditional_expression();
            ts5_is_context_sensitive(conditional.when_true) || ts5_is_context_sensitive(conditional.when_false)
        }
        Kind::BinaryExpression => {
            let binary = node.as_binary_expression();
            matches!(binary.operator_token.kind(), Kind::BarBarToken | Kind::QuestionQuestionToken)
                && (ts5_is_context_sensitive(binary.left) || ts5_is_context_sensitive(binary.right()))
        }
        Kind::PropertyAssignment => node.initializer().is_some_and(ts5_is_context_sensitive),
        Kind::ParenthesizedExpression => node.expression().is_some_and(ts5_is_context_sensitive),
        Kind::JsxAttributes => {
            node.properties().iter().any(|&p| ts5_is_context_sensitive(p))
                || node.parent().is_some_and(|parent| {
                    tsrs_ast::is_jsx_opening_element(parent)
                        && parent.parent().is_some_and(|element| {
                            element.children().nodes().iter().any(|&c| ts5_is_context_sensitive(c))
                        })
                })
        }
        Kind::JsxAttribute | Kind::JsxExpression => {
            let inner = if node.kind() == Kind::JsxAttribute { node.initializer() } else { node.expression() };
            inner.is_some_and(ts5_is_context_sensitive)
        }
        _ => false,
    }
}

/// TypeScript 5's `hasContextSensitiveParameters`.
fn ts5_has_context_sensitive_parameters(node: P<Node>) -> bool {
    if node.type_parameter_list().is_some() {
        return false;
    }
    node.parameters().iter().any(|p| p.type_node().is_none())
        || (node.kind() != Kind::ArrowFunction
            && node.parameters().first().is_none_or(|&first| !tsrs_ast::is_this_parameter(first)))
}

/// TypeScript 5's `hasContextSensitiveReturnExpression`.
fn ts5_has_context_sensitive_return_expression(node: P<Node>) -> bool {
    if node.type_parameter_list().is_some() || node.type_node().is_some() {
        return false;
    }
    let Some(body) = node.body() else { return false };
    if body.kind() != Kind::Block {
        return ts5_is_context_sensitive(body);
    }
    tsrs_ast::for_each_return_statement(body, |statement| statement.expression().is_some_and(ts5_is_context_sensitive))
}

/// rbxts-rs: TypeScript 5 types a function expression's implicit `this` once, from the first overload its
/// `chooseOverload` checks the function against in full: the first candidate whose arguments pass with the context
/// sensitive ones left out (as `anyFunctionType`), over the subtype pass, then the assignable one. That overload may
/// then be rejected (on the function's return type, say) and another chosen, but the `this` stays. TypeScript 7 does
/// not take a function that does not use `this` for context sensitive, so its contextual signature is the chosen
/// overload's: `over(cb: (this: void) => number)` declared before `over(cb: (this: Foo) => void)` gives `over(function
/// () {})` the `this: void` of the first (TypeScript 5 prints `function()`), where the chosen overload's `this` would
/// make it `function(self)`.
///
/// `Some` with that overload's contextual signature for `function`, an argument (in parentheses or not) of a call with
/// overloads; `None` (the contextual signature stands) for any other place, and for a call this does not model the way
/// TypeScript 5 resolves it: generic overloads (inferred with the function left out), spread arguments, optional
/// chains, type arguments, or another argument whose context sensitivity TypeScript 5 judges otherwise than as a whole
/// function (an object literal method of the same kind inside an argument, which TypeScript 5 checks as part of an
/// object of `anyFunctionType` members). When TypeScript 5 would then accept no overload, as when the first one
/// fixes a `this` the others refuse, it reports TS2769, which the TypeScript 7 checker cannot (the function has no
/// `this` there): the chosen overload's `this` is used then.
fn ts5_overload_contextual_signature(state: &mut TransformState<'_>, function: P<Node>) -> Option<Option<P<Signature>>> {
    if function.kind() != Kind::FunctionExpression {
        return None;
    }
    let mut argument = function;
    while let Some(parent) = argument.parent().filter(|p| p.kind() == Kind::ParenthesizedExpression) {
        argument = parent;
    }
    let call = argument.parent().filter(|p| p.kind() == Kind::CallExpression)?;
    let callee = call.expression()?;
    if tsrs_ast::is_optional_chain(call) || callee.kind() == Kind::SuperKeyword || callee.kind() == Kind::ImportKeyword {
        return None;
    }
    let args = call.arguments();
    let index = args.iter().position(|&a| a == argument)?;
    if call.type_argument_list().is_some() || args.iter().any(|a| a.kind() == Kind::SpreadElement) {
        return None;
    }
    let checker = &mut *state.checker;
    let callee_type = checker.check_non_null_expression(callee);
    let apparent = checker.get_apparent_type_exported(callee_type);
    let signatures = checker.get_signatures_of_type_exported(apparent, SignatureKind::Call).to_vec();
    let candidates = checker.reorder_candidates(&signatures, SignatureFlags::None);
    if candidates.len() < 2 || candidates.iter().any(|c| !c.type_parameters().is_empty()) {
        return None;
    }
    // the arguments TypeScript 5 leaves out as `anyFunctionType`; any other judged as TypeScript 7 judges it
    let mut any_function = Vec::with_capacity(args.len());
    for &arg in args {
        let ts5 = ts5_is_context_sensitive(arg);
        let inner = tsrs_ast::skip_parentheses(arg);
        let whole_function = matches!(inner.kind(), Kind::FunctionExpression | Kind::ArrowFunction);
        if ts5 && whole_function {
            any_function.push(true);
        } else if ts5 == checker.is_context_sensitive(arg) {
            any_function.push(false);
        } else {
            return None;
        }
    }
    for relation in [checker.subtype_relation, checker.assignable_relation] {
        for &candidate in &candidates {
            if !checker.has_correct_arity(call, args, candidate, false) {
                continue;
            }
            if applicable_leaving_out(checker, call, args, &any_function, candidate, relation)? {
                let parameter_type = checker.get_type_at_position_exported(candidate, index);
                checker.push_contextual_type(argument, Some(parameter_type), false);
                let contextual = checker.get_contextual_signature(function);
                checker.pop_contextual_type();
                return Some(contextual);
            }
        }
    }
    None
}

/// TypeScript 5's `getSignatureApplicabilityError` (no JSX) in `SkipContextSensitive` mode, with the arguments
/// `any_function` marks typed `anyFunctionType`. `None` when one of those falls in a rest parameter (its spread type
/// would check it in full).
fn applicable_leaving_out(
    checker: &mut tsrs_checker::Checker,
    call: P<Node>,
    args: &[P<Node>],
    any_function: &[bool],
    signature: P<Signature>,
    relation: P<Relation>,
) -> Option<bool> {
    if let Some(this_type) = checker.get_this_type_of_signature(signature)
        && this_type != checker.get_void_type()
        && !call.expression().is_some_and(tsrs_ast::is_super_property)
    {
        let this_argument = checker.get_this_argument_of_call(call);
        let this_argument_type = checker.get_this_argument_type(this_argument);
        if !checker.is_type_related_to(this_argument_type, this_type, relation) {
            return Some(false);
        }
    }
    let rest_type = checker.get_non_array_rest_type(signature);
    let arg_count =
        if rest_type.is_some() { (checker.get_parameter_count(signature) - 1).min(args.len()) } else { args.len() };
    if any_function[arg_count..].iter().any(|&f| f) {
        return None;
    }
    for (i, &arg) in args.iter().take(arg_count).enumerate() {
        if arg.kind() == Kind::OmittedExpression {
            continue;
        }
        let parameter_type = checker.get_type_at_position_exported(signature, i);
        let arg_type = if any_function[i] {
            checker.any_function_type
        } else {
            let t = checker.check_expression_with_contextual_type(arg, parameter_type, None, CheckMode::SkipContextSensitive);
            checker.get_regular_type_of_object_literal(t)
        };
        if !checker.is_type_related_to(arg_type, parameter_type, relation) {
            return Some(false);
        }
    }
    if let Some(rest_type) = rest_type {
        let spread_type =
            checker.get_spread_argument_type(args, arg_count, args.len(), rest_type, None, CheckMode::SkipContextSensitive);
        if !checker.is_type_related_to(spread_type, rest_type, relation) {
            return Some(false);
        }
    }
    Some(true)
}

fn is_method_inner(state: &mut TransformState<'_>, node: P<Node>, t: P<Type>) -> bool {
    let mut has_method_definition = false;
    let mut has_callback_definition = false;

    for &signature in state.checker.get_signatures_of_type_exported(t, SignatureKind::Call) {
        let this_parameter = signature.this_parameter().or_else(|| ts5_contextual_this_parameter(state, signature));
        if let Some(this_parameter) = this_parameter {
            let this_type = state
                .checker
                .get_type_of_symbol_at_location(this_parameter, Some(node))
                .unwrap_or_else(|| state.checker.get_error_type());
            if can_change_receiver_convention(state, this_type)
                && state.multi_transform_state.is_reported_by_no_unstable_this_type.insert(node)
            {
                state.add_diagnostic(errors::no_unstable_this_type(node));
            }
            if !this_type.flags().intersects(TypeFlags::Void) {
                has_method_definition = true;
            } else {
                has_callback_definition = true;
            }
        } else if let Some(declaration) = signature.declaration() {
            if is_method_declaration(declaration) {
                has_method_definition = true;
            } else {
                has_callback_definition = true;
            }
        }
    }

    if has_method_definition && has_callback_definition {
        state.add_diagnostic(errors::no_mixed_type_call(node));
    }

    has_method_definition
}

/// `isMethodFromType`
pub fn is_method_from_type(state: &mut TransformState<'_>, node: P<Node>, t: P<Type>) -> bool {
    let mut result = false;
    walk_types(state, t, &mut |state, t| {
        // `result ||= …` skips (and does not cache) the check once a method was found
        if t.symbol().is_some() && !result {
            let value = match state.multi_transform_state.is_method_cache.get(&t) {
                Some(&cached) => cached,
                None => {
                    let value = is_method_inner(state, node, t);
                    state.multi_transform_state.is_method_cache.insert(t, value);
                    value
                }
            };
            result |= value;
        }
    });
    result
}

/// `isMethod`: by the node's type.
pub fn is_method(state: &mut TransformState<'_>, node: P<Node>) -> bool {
    let t = state.get_type(node);
    is_method_from_type(state, node, t)
}
