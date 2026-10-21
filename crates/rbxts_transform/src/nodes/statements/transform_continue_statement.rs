//! `nodes/statements/transformContinueStatement.ts`

use rbxts_luau::{Stmt, create};
use tsrs_ast::Node;
use tsrs_core::P;

use crate::classes::transform_state::{TransformState, TryUse};
use crate::shared::diagnostics::errors;
use crate::util::is_blocked_by_try_statement::is_break_blocked_by_try_statement;

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

    vec![create::continue_stmt()]
}
