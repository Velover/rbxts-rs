//! `nodes/transformParameters.ts`

use rbxts_luau::{AnyIdentifier, Expr, Stmt, create, globals};
use tsrs_ast::{Kind, Node};
use tsrs_core::P;

use crate::classes::prereqs::{as_any_identifier, temp_id};
use crate::classes::transform_state::TransformState;
use crate::nodes::expressions::transform_identifier::transform_identifier_defined;
use crate::nodes::transform_initializer::transform_initializer;
use crate::util::array_binding_pattern_contains_hoists::array_like_expression_contains_spread;
use crate::util::is_method::is_method;
use crate::util::symbols::validate_identifier;

/// The parameters of a lowered function.
pub struct TransformedParameters {
    pub parameters: Vec<AnyIdentifier>,
    pub statements: Vec<Stmt>,
    pub has_dot_dot_dot: bool,
}

/// `transformParameters`: the Luau parameter list (`self` first for methods), with defaults, rest parameters and
/// destructuring as statements.
pub fn transform_parameters(state: &mut TransformState<'_>, node: P<Node>) -> TransformedParameters {
    let mut parameters: Vec<AnyIdentifier> = Vec::new();
    let mut statements: Vec<Stmt> = Vec::new();
    let mut has_dot_dot_dot = false;

    if is_method(state, node) {
        parameters.push(as_any_identifier(globals::self_()));
    }

    for &parameter in node.parameters() {
        let name = parameter.name().expect("a parameter has a name");
        if tsrs_ast::is_this_identifier(name) {
            continue;
        }
        let declaration = parameter.as_parameter_declaration();
        let is_rest = declaration.dot_dot_dot_token().is_some();

        if is_rest && name.kind() == Kind::ArrayBindingPattern && !array_like_expression_contains_spread(name) {
            // `...[a, b, c]: [A, B, C]` becomes `(a, b, c)`
            state.unsupported_subtree(parameter, "rest parameter array binding pattern");
            continue;
        }

        let param_id: Expr = if name.kind() == Kind::Identifier {
            let id = transform_identifier_defined(state, name);
            validate_identifier(state, name);
            id
        } else {
            Expr::TemporaryIdentifier(temp_id("param"))
        };

        if is_rest {
            has_dot_dot_dot = true;
            statements.push(create::local(
                as_any_identifier(param_id.clone()),
                Some(create::array(vec![create::var_args()])),
            ));
        } else {
            parameters.push(as_any_identifier(param_id.clone()));
        }

        if let Some(initializer) = declaration.initializer() {
            statements.push(transform_initializer(state, param_id.clone(), initializer));
        }

        // destructuring
        if name.kind() != Kind::Identifier {
            state.unsupported_subtree(name, &format!("{:?} parameter", name.kind()));
        }
    }

    TransformedParameters { parameters, statements, has_dot_dot_dot }
}
