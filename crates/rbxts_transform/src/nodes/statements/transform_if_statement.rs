//! `nodes/statements/transformIfStatement.ts`

use rbxts_luau::{ElseBody, IfStatement, Stmt};
use tsrs_ast::{Kind, Node};
use tsrs_core::P;

use crate::classes::prereqs::Prereqs;
use crate::classes::transform_state::TransformState;
use crate::nodes::expressions::transform_expression::transform_expression;
use crate::nodes::transform_statement_list::transform_statement_list;
use crate::util::create_truthiness_checks::create_truthiness_checks;
use crate::util::symbols::get_statements;

/// `transformIfStatementInner`
pub fn transform_if_statement_inner(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    node: P<Node>,
) -> IfStatement {
    let if_statement = node.as_if_statement();
    let expression = transform_expression(state, prereqs, if_statement.expression);
    let condition = create_truthiness_checks(state, prereqs, expression, if_statement.expression);

    let then_statement = if_statement.then_statement;
    let statements = transform_statement_list(state, Some(then_statement), &get_statements(then_statement));

    let else_body = match if_statement.else_statement {
        None => ElseBody::default(),
        Some(else_statement) if else_statement.kind() == Kind::IfStatement => {
            let mut else_if_prereqs = Prereqs::new();
            let else_if = transform_if_statement_inner(state, &mut else_if_prereqs, else_statement);
            if else_if_prereqs.statements.is_empty() {
                ElseBody::If(Box::new(else_if))
            } else {
                let mut else_if_statements = else_if_prereqs.statements;
                else_if_statements.push(Stmt::If(Box::new(else_if)));
                ElseBody::Block(else_if_statements)
            }
        }
        Some(else_statement) => {
            ElseBody::Block(transform_statement_list(state, Some(else_statement), &get_statements(else_statement)))
        }
    };

    IfStatement { condition, statements, else_body }
}

/// `transformIfStatement`
pub fn transform_if_statement(state: &mut TransformState<'_>, node: P<Node>) -> Vec<Stmt> {
    let mut prereqs = Prereqs::new();
    let statement = transform_if_statement_inner(state, &mut prereqs, node);
    let mut statements = prereqs.statements;
    statements.push(Stmt::If(Box::new(statement)));
    statements
}
