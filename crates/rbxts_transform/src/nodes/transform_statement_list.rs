//! `nodes/transformStatementList.ts`

use rbxts_luau::{AnyIdentifier, AssignmentOperator, Stmt, create};
use rustc_hash::FxHashMap;
use tsrs_ast::{Kind, Node};
use tsrs_core::P;

use crate::classes::transform_state::TransformState;
use crate::nodes::statements::transform_statement::transform_statement;
use crate::util::create_hoist_declaration::create_hoist_declaration;
use crate::util::traversal::{get_start, source_file_of};

/// Namespace export handling: the container and the names each statement exports.
pub struct ExportInfo<'m> {
    pub id: AnyIdentifier,
    pub mapping: &'m FxHashMap<P<Node>, Vec<String>>,
}

/// The full start of a node's last token, when that token is not part of `last_statement`
/// (`getLastToken` and its `isNodeDescendantOf` check).
///
/// rbxts-rs: tsrs has no token children, so the last token is found from the syntax: a block's `}` follows its last
/// statement (or its `{`), a source file ends with its end-of-file token, and anything else is scanned. With no
/// `last_statement` (an empty slice of a block, as `transformClassConstructor` passes), the block's own last statement
/// still decides where its `}` is.
fn last_token_pos(node: P<Node>, last_statement: Option<P<Node>>) -> Option<u32> {
    match node.kind() {
        Kind::Block | Kind::ModuleBlock => Some(match last_statement {
            Some(last) => last.end().source_offset()?,
            None => match node.statements().last() {
                Some(last) => last.end().source_offset()?,
                None => get_start(node) + 1,
            },
        }),
        Kind::SourceFile => node.as_source_file().end_of_file_token.pos().source_offset(),
        _ => {
            let file = source_file_of(node);
            let from = match last_statement {
                Some(last) => last.end().source_offset()?,
                None => node.pos().source_offset()?,
            };
            let end = node.end().source_offset()?;
            let mut scanner = tsrs_scanner::get_scanner_for_source_file(file, from);
            let mut last = None;
            while scanner.token() != Kind::EndOfFile && scanner.token_end() <= end {
                last = Some(scanner.token_full_start());
                if scanner.token_end() == end {
                    break;
                }
                scanner.scan();
            }
            last
        }
    }
}

/// `transformStatementList`: statements with their leading comments and hoisted declarations, stopping after a
/// final statement (`return`, `break`, `continue`).
pub fn transform_statement_list(
    state: &mut TransformState<'_>,
    parent: Option<P<Node>>,
    statements: &[P<Node>],
) -> Vec<Stmt> {
    transform_statement_list_with_exports(state, parent, statements, None)
}

/// `transformStatementList` with namespace export information.
pub fn transform_statement_list_with_exports(
    state: &mut TransformState<'_>,
    parent: Option<P<Node>>,
    statements: &[P<Node>],
    export_info: Option<&ExportInfo<'_>>,
) -> Vec<Stmt> {
    let mut result = Vec::new();
    let remove_comments = state.compiler_options.remove_comments.is_true();

    for &statement in statements {
        let transformed_statements = transform_statement(state, statement);

        // the statement's leading comments
        if !remove_comments {
            result.extend(state.get_leading_comments(statement));
        }

        // hoisting is the use of a variable before it was declared
        if let Some(hoist_declaration) = create_hoist_declaration(state, statement) {
            result.push(hoist_declaration);
        }

        let is_final = transformed_statements.last().is_some_and(Stmt::is_final);
        result.extend(transformed_statements);
        if is_final {
            break;
        }

        // namespace export handling
        if let Some(export_info) = export_info
            && let Some(names) = export_info.mapping.get(&statement)
        {
            for name in names {
                result.push(create::assignment(
                    create::property(export_info.id.clone().into(), name.clone()),
                    AssignmentOperator::Assign,
                    create::id(name.clone()),
                ));
            }
        }
    }

    if !remove_comments {
        let last_token = match statements.last() {
            Some(&last_statement) => {
                let owner = last_statement.parent().expect("a statement has a parent");
                last_token_pos(owner, Some(last_statement))
            }
            // if there are no statements, there still might be an EOF or } token to look for
            None => parent.and_then(|p| last_token_pos(p, None)),
        };
        if let Some(pos) = last_token {
            result.extend(state.get_leading_comments_at(pos));
        }
    }

    result
}
