//! `nodes/statements/transformSwitchStatement.ts`: a `repeat … until true` block of `if` statements, with a flag for
//! fallthrough.

use rbxts_luau::{
    AssignmentOperator, BinaryOperator, ElseBody, Expr, Stmt, TemporaryIdentifier, UnaryOperator, create,
};
use tsrs_ast::{Kind, Node, SymbolFlags};
use tsrs_core::P;

use crate::classes::prereqs::{Prereqs, temp_id};
use crate::classes::transform_state::TransformState;
use crate::nodes::expressions::transform_expression::transform_expression;
use crate::nodes::transform_statement_list::transform_statement_list;
use crate::util::create_hoist_declaration::create_hoist_declaration;
use crate::util::expression_might_mutate::expression_might_mutate;
use crate::util::traversal::skip_downwards;

fn can_compare_identifier_directly(state: &mut TransformState<'_>, node: P<Node>, expression: &Expr) -> bool {
    let switch = node.as_switch_statement();
    let identifier = skip_downwards(switch.expression);
    if identifier.kind() != Kind::Identifier || !matches!(expression, Expr::Identifier(_)) {
        return false;
    }

    // declarations shared by case bodies can be hoisted ahead of their comparisons
    let resolved = state.checker.resolve_name_exported(identifier.text(), switch.case_block, SymbolFlags::Value, false);
    if resolved != state.checker.get_symbol_at_location_exported(identifier) {
        return false;
    }

    // literal labels cannot mutate the operand, and fallthrough skips later comparisons
    switch.case_block.as_case_block().clauses.nodes().iter().all(|&clause| {
        if clause.kind() == Kind::DefaultClause {
            return true;
        }
        let label = skip_downwards(clause.expression().expect("a case clause has an expression"));
        tsrs_ast::is_string_literal_like(label)
            || matches!(label.kind(), Kind::NumericLiteral | Kind::TrueKeyword | Kind::FalseKeyword)
    })
}

/// The condition of a case clause, and the statements it needs before it.
struct CaseClauseExpression {
    condition: Expr,
    prereq_statements: Vec<Stmt>,
}

fn transform_case_clause_expression(
    state: &mut TransformState<'_>,
    case_clause_expression: P<Node>,
    switch_expression: &Expr,
    fall_through_flag_id: &TemporaryIdentifier,
    can_fall_through_to: bool,
) -> CaseClauseExpression {
    let case_value_id = temp_id("caseValue");
    let mut case_prereqs = Prereqs::new();
    let expression = transform_expression(state, &mut case_prereqs, case_clause_expression);
    let mut prereq_statements = case_prereqs.statements;
    let case_value_might_mutate = expression_might_mutate(state, &expression, Some(case_clause_expression));

    let compared = if case_value_might_mutate {
        prereq_statements.push(create::local(case_value_id.clone(), Some(expression)));
        Expr::TemporaryIdentifier(case_value_id)
    } else {
        expression
    };
    let mut condition = create::binary(switch_expression.clone(), BinaryOperator::Eq, compared);

    if can_fall_through_to {
        let fall_through_flag = Expr::TemporaryIdentifier(fall_through_flag_id.clone());
        if !prereq_statements.is_empty() {
            let no_fall_through_condition = create::unary(UnaryOperator::Not, fall_through_flag.clone());
            prereq_statements.push(create::assignment(
                fall_through_flag.clone(),
                AssignmentOperator::Assign,
                condition,
            ));
            prereq_statements =
                vec![create::if_stmt(no_fall_through_condition, prereq_statements, ElseBody::default())];
            condition = fall_through_flag;
        } else {
            condition = create::binary(fall_through_flag, BinaryOperator::Or, condition);
        }
    }

    CaseClauseExpression { condition, prereq_statements }
}

/// A transformed case clause.
struct CaseClause {
    can_fall_through_from: bool,
    prereqs: Vec<Stmt>,
    clause_statements: Vec<Stmt>,
}

fn transform_case_clause(
    state: &mut TransformState<'_>,
    node: P<Node>,
    switch_expression: &Expr,
    fall_through_flag_id: &TemporaryIdentifier,
    can_fall_through_to: bool,
    should_update_fall_through_flag: bool,
) -> CaseClause {
    let CaseClauseExpression { condition, prereq_statements } = transform_case_clause_expression(
        state,
        node.expression().expect("a case clause has an expression"),
        switch_expression,
        fall_through_flag_id,
        can_fall_through_to,
    );

    let clause_body = node.statements();
    let non_empty_statements: Vec<P<Node>> =
        clause_body.iter().copied().filter(|s| s.kind() != Kind::EmptyStatement).collect();
    let mut statements = match non_empty_statements.as_slice() {
        [first] if first.kind() == Kind::Block => transform_statement_list(state, Some(*first), first.statements()),
        _ => transform_statement_list(state, Some(node), clause_body),
    };

    let can_fall_through_from = statements.last().is_none_or(|last| !last.is_final());
    if can_fall_through_from && should_update_fall_through_flag {
        statements.push(create::assignment(
            Expr::TemporaryIdentifier(fall_through_flag_id.clone()),
            AssignmentOperator::Assign,
            Expr::True,
        ));
    }

    let mut clause_statements = Vec::new();
    if let Some(hoist_declaration) = create_hoist_declaration(state, node) {
        clause_statements.push(hoist_declaration);
    }
    clause_statements.push(create::if_stmt(condition, statements, ElseBody::default()));

    CaseClause { can_fall_through_from, prereqs: prereq_statements, clause_statements }
}

/// `transformSwitchStatement`
pub fn transform_switch_statement(state: &mut TransformState<'_>, node: P<Node>) -> Vec<Stmt> {
    let switch = node.as_switch_statement();
    let mut prereqs = Prereqs::new();
    let switch_expression = transform_expression(state, &mut prereqs, switch.expression);
    let needs_capture = expression_might_mutate(state, &switch_expression, Some(switch.expression))
        && !can_compare_identifier_directly(state, node, &switch_expression);
    let expression = if needs_capture {
        prereqs.push_to_var(switch_expression, "exp")
    } else {
        prereqs.push_to_var_if_complex(switch_expression, "exp")
    };
    let mut result = prereqs.statements;
    let fall_through_flag_id = temp_id("fallthrough");

    let mut is_fall_through_flag_needed = false;

    let mut statements = Vec::new();
    let mut can_fall_through_to = false;
    let clauses = switch.case_block.as_case_block().clauses.nodes();
    for (i, &case_clause_node) in clauses.iter().enumerate() {
        if case_clause_node.kind() == Kind::CaseClause {
            let should_update_fall_through_flag = i < clauses.len() - 1 && clauses[i + 1].kind() == Kind::CaseClause;
            let CaseClause { can_fall_through_from, prereqs, clause_statements } = transform_case_clause(
                state,
                case_clause_node,
                &expression,
                &fall_through_flag_id,
                can_fall_through_to,
                should_update_fall_through_flag,
            );

            statements.extend(prereqs);
            statements.extend(clause_statements);

            can_fall_through_to = can_fall_through_from;

            if can_fall_through_from {
                is_fall_through_flag_needed = true;
            }
        } else {
            statements.extend(transform_statement_list(state, Some(case_clause_node), case_clause_node.statements()));
            break;
        }
    }

    if is_fall_through_flag_needed {
        statements.insert(0, create::local(fall_through_flag_id, Some(Expr::False)));
    }

    result.push(create::repeat_stmt(statements, Expr::True));
    result
}
