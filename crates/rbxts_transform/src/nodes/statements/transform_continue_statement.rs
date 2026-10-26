//! `nodes/statements/transformContinueStatement.ts`

use rbxts_luau::{AssignmentOperator, Expr, Stmt, create};
use tsrs_ast::{Kind, Node};
use tsrs_core::P;

use crate::classes::prereqs::temp_id;
use crate::classes::transform_state::{TransformState, TryUse};
use crate::shared::diagnostics::errors;
use crate::util::is_blocked_by_try_statement::is_break_blocked_by_try_statement;
use crate::util::traversal::get_ancestor;

/// rbxts-rs: the Luau for continuing the loop around `node` (a statement), at `node`'s place.
///
/// Fixes roblox-ts emitting `continue` there even inside a switch: a switch is a `repeat … until true` block, so the
/// `continue` continued that block and left the switch like `break`, and the rest of the loop body still ran. Inside a
/// switch, this sets the switch's flag and breaks out of its block; the switch then continues the loop after its
/// block (`transform_switch_statement`, which uses this again, so nested switches pass it on). Labels are errors, so
/// the loop is always the innermost one.
pub fn create_continue(state: &mut TransformState<'_>, node: P<Node>) -> Vec<Stmt> {
    let target = node.parent().and_then(|parent| {
        get_ancestor(parent, |a| {
            tsrs_ast::is_iteration_statement(a, false)
                || a.kind() == Kind::SwitchStatement
                || tsrs_ast::is_function_like(a)
        })
    });
    match target {
        Some(switch) if switch.kind() == Kind::SwitchStatement => {
            let flag = state.switch_continue_flags.entry(switch).or_insert_with(|| temp_id("continue")).clone();
            vec![
                create::assignment(Expr::TemporaryIdentifier(flag), AssignmentOperator::Assign, Expr::True),
                create::break_stmt(),
            ]
        }
        _ => vec![create::continue_stmt()],
    }
}

/// `transformContinueStatement`: `continue`, or `return TS.TRY_CONTINUE` inside a try statement's function.
pub fn transform_continue_statement(state: &mut TransformState<'_>, node: P<Node>) -> Vec<Stmt> {
    if let Some(label) = node.as_continue_statement().label {
        state.add_diagnostic(errors::no_labeled_statement(label));
        return Vec::new();
    }

    if is_break_blocked_by_try_statement(node) {
        state.mark_try_uses(TryUse::Continue);
        return vec![create::return_stmt(state.ts(node, "TRY_CONTINUE"))];
    }

    create_continue(state, node)
}
