//! rbxts-rs only: what else in an unsupported node's subtree is unsupported.
//!
//! When the transform meets a node kind it does not handle yet, it does not transform the node's children, so the
//! kinds nested inside (a class's methods using macros, a loop body with destructuring) would go unreported. The
//! parity report's histogram ranks the work by the files each kind blocks, so the subtree is probed with the same
//! classification the transform applies, without transforming anything.

use tsrs_ast::{Kind, Node};
use tsrs_core::P;

use crate::classes::transform_state::TransformState;
use crate::nodes::expressions::transform_expression::kind_name;
use crate::util::types::get_first_defined_symbol;

/// Statement kinds the transform reports as unsupported.
pub fn is_unsupported_statement_kind(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::ClassDeclaration | Kind::DoStatement | Kind::ForOfStatement | Kind::ForStatement | Kind::WhileStatement
    )
}

/// Expression kinds the transform reports as unsupported.
pub fn is_unsupported_expression_kind(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::ArrayLiteralExpression
            | Kind::ClassExpression
            | Kind::JsxElement
            | Kind::JsxExpression
            | Kind::JsxFragment
            | Kind::JsxSelfClosingElement
            | Kind::NewExpression
            | Kind::ObjectLiteralExpression
            | Kind::SpreadElement
    )
}

fn is_binding_pattern(node: P<Node>) -> bool {
    matches!(node.kind(), Kind::ObjectBindingPattern | Kind::ArrayBindingPattern)
}

/// The macro a call expands, as the transform labels it.
fn macro_label(state: &mut TransformState<'_>, call: P<Node>) -> Option<String> {
    if tsrs_ast::is_import_call(call) {
        return None;
    }
    let callee = call.expression()?;
    if callee.kind() == Kind::SuperKeyword {
        return None;
    }
    let callee_type = state.get_type(callee);
    let exp_type = state.checker.get_non_optional_type(callee_type);
    let symbol = get_first_defined_symbol(state, exp_type)?;
    let inner = crate::util::traversal::skip_downwards(callee);
    if matches!(inner.kind(), Kind::PropertyAccessExpression | Kind::ElementAccessExpression) {
        let r#macro = state.macro_manager.get_property_call_macro(state.checker, symbol)?;
        r#macro.transform.is_none().then(|| format!("property call macro {}", r#macro.name))
    } else {
        let r#macro = state.macro_manager.get_call_macro(state.checker, symbol)?;
        r#macro.transform.is_none().then(|| format!("call macro {}", r#macro.name))
    }
}

/// The unsupported label of one node, as the transform would report it there.
fn classify(state: &mut TransformState<'_>, node: P<Node>) -> Option<String> {
    let kind = node.kind();
    if is_unsupported_statement_kind(kind) || is_unsupported_expression_kind(kind) {
        return Some(kind_name(kind));
    }
    match kind {
        Kind::CallExpression => macro_label(state, node),
        Kind::Identifier => {
            // only the names of identifier macros can resolve to one (names of declarations and properties resolve
            // to other symbols)
            if !crate::macros::IDENTIFIER_MACROS.iter().any(|(name, _)| *name == node.text()) {
                return None;
            }
            let symbol = state.checker.get_symbol_at_location_exported(node)?;
            let r#macro = state.macro_manager.get_identifier_macro(state.checker, symbol)?;
            r#macro.transform.is_none().then(|| format!("identifier macro {}", r#macro.name))
        }
        Kind::BinaryExpression => {
            let binary = node.as_binary_expression();
            if tsrs_ast::is_assignment_operator(binary.operator_token.kind()) {
                match binary.left.kind() {
                    Kind::ArrayLiteralExpression => Some("array destructuring assignment".to_owned()),
                    Kind::ObjectLiteralExpression => Some("object destructuring assignment".to_owned()),
                    _ => None,
                }
            } else {
                None
            }
        }
        Kind::VariableDeclaration => {
            let name = node.name()?;
            (is_binding_pattern(name) && !name.elements().is_empty()).then(|| format!("{:?} variable", name.kind()))
        }
        Kind::Parameter => {
            let name = node.name()?;
            if !is_binding_pattern(name) {
                return None;
            }
            let is_rest = node.as_parameter_declaration().dot_dot_dot_token().is_some();
            Some(if is_rest && name.kind() == Kind::ArrayBindingPattern {
                "rest parameter array binding pattern".to_owned()
            } else {
                format!("{:?} parameter", name.kind())
            })
        }
        _ => None,
    }
}

fn visit(state: &mut TransformState<'_>, node: P<Node>, out: &mut Vec<String>) {
    // types and JSDoc are not transformed
    if tsrs_ast::is_part_of_type_node(node) {
        return;
    }
    if let Some(label) = classify(state, node) {
        out.push(label);
    }
    node.for_each_child(&mut |child| {
        visit(state, child, out);
        false
    });
}

/// Records the unsupported kinds inside `node` (not `node` itself) for the file's report, without diagnostics.
pub fn probe_unsupported_children(state: &mut TransformState<'_>, node: P<Node>) {
    let mut found = Vec::new();
    node.for_each_child(&mut |child| {
        visit(state, child, &mut found);
        false
    });
    state.diagnostics.unsupported.extend(found);
}
