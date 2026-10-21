//! `nodes/transformOptionalChain.ts`: property accesses, element accesses and calls, flattened into a chain so
//! that `?.` links get their nil checks.

use rbxts_luau::{AssignmentOperator, BinaryOperator, ElseBody, Expr, Stmt, create, globals};
use tsrs_ast::{Kind, Node};
use tsrs_core::P;

use crate::classes::prereqs::Prereqs;
use crate::classes::transform_state::TransformState;
use crate::nodes::expressions::transform_call_expression::{
    transform_call_expression_inner, transform_element_call_expression_inner, transform_property_call_expression_inner,
};
use crate::nodes::expressions::transform_element_access_expression::transform_element_access_expression_inner;
use crate::nodes::expressions::transform_expression::transform_expression;
use crate::nodes::expressions::transform_property_access_expression::transform_property_access_expression_inner;
use crate::shared::diagnostics::errors;
use crate::util::add_one_if_array_type::add_one_if_array_type;
use crate::util::convert_to_indexable_expression::convert_to_indexable_expression;
use crate::util::ensure_transform_order::ensure_transform_order;
use crate::util::is_method::is_method;
use crate::util::symbols::is_used_as_statement;
use crate::util::traversal::skip_downwards;
use crate::util::types::get_first_defined_symbol;
use crate::util::wrap_return_if_lua_tuple::wrap_return_if_lua_tuple;

/// One link of a chain (`ChainItem`).
#[derive(Clone, Copy)]
pub enum ChainItemKind {
    PropertyAccess { name: &'static str },
    ElementAccess { expression: P<Node> },
    Call,
    PropertyCall { expression: P<Node>, name: &'static str, call_optional: bool },
    ElementCall { expression: P<Node>, argument_expression: P<Node>, call_optional: bool },
}

/// `ChainItem`
#[derive(Clone, Copy)]
pub struct ChainItem {
    pub kind: ChainItemKind,
    pub node: P<Node>,
    pub optional: bool,
}

impl ChainItem {
    fn args(&self) -> &'static [P<Node>] {
        self.node.arguments()
    }

    fn is_compound_call(&self) -> bool {
        matches!(self.kind, ChainItemKind::PropertyCall { .. } | ChainItemKind::ElementCall { .. })
    }

    fn call_optional(&self) -> bool {
        match self.kind {
            ChainItemKind::PropertyCall { call_optional, .. } | ChainItemKind::ElementCall { call_optional, .. } => {
                call_optional
            }
            _ => false,
        }
    }

    /// The access expression of a compound call.
    fn access(&self) -> P<Node> {
        match self.kind {
            ChainItemKind::PropertyCall { expression, .. } | ChainItemKind::ElementCall { expression, .. } => {
                expression
            }
            _ => unreachable!("not a compound call"),
        }
    }
}

fn has_question_dot(node: P<Node>) -> bool {
    node.question_dot_token().is_some()
}

/// `flattenOptionalChain`: the chain items from the innermost receiver out, and that receiver.
pub fn flatten_optional_chain(mut expression: P<Node>) -> (Vec<ChainItem>, P<Node>) {
    let mut chain = Vec::new();
    loop {
        match expression.kind() {
            Kind::PropertyAccessExpression => {
                let access = expression.as_property_access_expression();
                chain.push(ChainItem {
                    kind: ChainItemKind::PropertyAccess { name: access.name.text() },
                    node: expression,
                    optional: has_question_dot(expression),
                });
                expression = access.expression;
            }
            Kind::ElementAccessExpression => {
                let access = expression.as_element_access_expression();
                chain.push(ChainItem {
                    kind: ChainItemKind::ElementAccess { expression: access.argument_expression },
                    node: expression,
                    optional: has_question_dot(expression),
                });
                expression = access.expression;
            }
            Kind::CallExpression => {
                // this is a bit of a mess..
                let sub_exp = skip_downwards(expression.expression().expect("a call has a callee"));
                let call_optional = has_question_dot(expression);
                match sub_exp.kind() {
                    Kind::PropertyAccessExpression => {
                        let access = sub_exp.as_property_access_expression();
                        chain.push(ChainItem {
                            kind: ChainItemKind::PropertyCall {
                                expression: sub_exp,
                                name: access.name.text(),
                                call_optional,
                            },
                            node: expression,
                            optional: has_question_dot(sub_exp),
                        });
                        expression = access.expression;
                    }
                    Kind::ElementAccessExpression => {
                        let access = sub_exp.as_element_access_expression();
                        chain.push(ChainItem {
                            kind: ChainItemKind::ElementCall {
                                expression: sub_exp,
                                argument_expression: access.argument_expression,
                                call_optional,
                            },
                            node: expression,
                            optional: has_question_dot(sub_exp),
                        });
                        expression = access.expression;
                    }
                    _ => {
                        chain.push(ChainItem { kind: ChainItemKind::Call, node: expression, optional: call_optional });
                        expression = sub_exp;
                    }
                }
            }
            _ => break,
        }
    }
    chain.reverse();
    (chain, expression)
}

fn transform_chain_item(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    base_expression: Expr,
    item: &ChainItem,
) -> Expr {
    match item.kind {
        ChainItemKind::PropertyAccess { name } => {
            transform_property_access_expression_inner(state, prereqs, item.node, base_expression, name)
        }
        ChainItemKind::ElementAccess { expression } => {
            transform_element_access_expression_inner(state, prereqs, item.node, base_expression, expression)
        }
        ChainItemKind::Call => transform_call_expression_inner(state, prereqs, item.node, base_expression, item.args()),
        ChainItemKind::PropertyCall { expression, name, .. } => transform_property_call_expression_inner(
            state,
            prereqs,
            item.node,
            expression,
            base_expression,
            name,
            item.args(),
        ),
        ChainItemKind::ElementCall { expression, argument_expression, .. } => transform_element_call_expression_inner(
            state,
            prereqs,
            item.node,
            expression,
            base_expression,
            argument_expression,
            item.args(),
        ),
    }
}

fn temp_name_for(node: P<Node>) -> String {
    match node.parent() {
        Some(parent)
            if parent.kind() == Kind::VariableDeclaration
                && parent.name().is_some_and(|n| n.kind() == Kind::Identifier) =>
        {
            parent.name().unwrap().text().to_owned()
        }
        _ => "result".to_owned(),
    }
}

fn same_temp(a: &Expr, b: &Expr) -> bool {
    matches!((a, b), (Expr::TemporaryIdentifier(x), Expr::TemporaryIdentifier(y)) if x.id == y.id)
}

fn create_or_set_temp_id(prereqs: &mut Prereqs, temp_id: Option<Expr>, expression: Expr, node: P<Node>) -> Expr {
    match temp_id {
        None => prereqs.push_to_var(expression, &temp_name_for(node)),
        Some(temp_id) => {
            if !same_temp(&temp_id, &expression) {
                prereqs.push(create::assignment(temp_id.clone(), AssignmentOperator::Assign, expression));
            }
            temp_id
        }
    }
}

fn create_nil_check(temp_id: Expr, statements: Vec<Stmt>) -> Stmt {
    create::if_stmt(create::binary(temp_id, BinaryOperator::Ne, Expr::Nil), statements, ElseBody::default())
}

fn transform_optional_chain_inner(
    state: &mut TransformState<'_>,
    prereqs: &mut Prereqs,
    chain: &[ChainItem],
    mut base_expression: Expr,
    mut temp_id: Option<Expr>,
    index: usize,
) -> Expr {
    let Some(item) = chain.get(index) else { return base_expression };
    let last_node = chain[chain.len() - 1].node;
    if item.optional || (item.is_compound_call() && item.call_optional()) {
        let mut is_method_call = false;
        let mut is_super_call = false;
        let mut self_param: Option<Expr> = None;

        if item.is_compound_call() {
            is_method_call = is_method(state, item.access());
            is_super_call = tsrs_ast::is_super_property(item.access());

            if item.call_optional() && is_method_call && !is_super_call {
                let param = prereqs.push_to_var(base_expression, "self");
                self_param = Some(param.clone());
                base_expression = param;
            }

            if item.optional {
                let temp = create_or_set_temp_id(prereqs, temp_id.take(), base_expression, last_node);
                temp_id = Some(temp.clone());
                base_expression = temp;
            }

            if item.call_optional() {
                match item.kind {
                    ChainItemKind::PropertyCall { name, .. } => {
                        base_expression = create::property(convert_to_indexable_expression(base_expression), name);
                    }
                    ChainItemKind::ElementCall { expression, argument_expression, .. } => {
                        let exp_type = state.get_type(expression.as_element_access_expression().expression);
                        let argument = transform_expression(state, prereqs, argument_expression);
                        let index = add_one_if_array_type(state, exp_type, argument);
                        base_expression =
                            create::computed_index(convert_to_indexable_expression(base_expression), index);
                    }
                    _ => unreachable!(),
                }
            }
        }

        // receiver initialization may need its own nil check before the optional call
        let mut receiver_prereqs = Prereqs::new();
        let temp = create_or_set_temp_id(&mut receiver_prereqs, temp_id.take(), base_expression, last_node);
        temp_id = Some(temp.clone());

        let mut branch_prereqs = Prereqs::new();
        let new_expression: Option<Expr> = if item.is_compound_call() && item.call_optional() {
            let callee = item.node.expression().expect("a call has a callee");
            let callee_type = state.get_type(callee);
            let exp_type = state.checker.get_non_optional_type(callee_type);
            let symbol = get_first_defined_symbol(state, exp_type);
            let r#macro = symbol.and_then(|s| state.macro_manager.get_property_call_macro(state.checker, s));
            if r#macro.is_some() {
                state.add_diagnostic(errors::no_optional_macro_call(item.node));
                None
            } else {
                let mut args = ensure_transform_order(state, &mut branch_prereqs, item.args());
                if is_method_call {
                    if is_super_call {
                        args.insert(0, globals::self_());
                    } else {
                        args.insert(0, self_param.clone().expect("a method call has a self parameter"));
                    }
                }
                Some(wrap_return_if_lua_tuple(state, item.node, create::call(temp.clone(), args)))
            }
        } else {
            Some(transform_chain_item(state, &mut branch_prereqs, temp.clone(), item))
        };
        let new_value = match new_expression {
            Some(e) => transform_optional_chain_inner(state, &mut branch_prereqs, chain, e, temp_id.clone(), index + 1),
            None => Expr::None,
        };
        let is_used = !new_value.is_none() && !is_used_as_statement(item.node);

        if !same_temp(&temp, &new_value) && is_used {
            branch_prereqs.push(create::assignment(temp.clone(), AssignmentOperator::Assign, new_value));
        } else if new_value.is_call() {
            branch_prereqs.push(create::call_statement(new_value));
        }

        receiver_prereqs.push(create_nil_check(temp.clone(), branch_prereqs.statements));

        if item.is_compound_call() && item.optional && item.call_optional() {
            prereqs.push(create_nil_check(temp.clone(), receiver_prereqs.statements));
        } else {
            prereqs.push_list(receiver_prereqs.statements);
        }

        if is_used { temp } else { Expr::None }
    } else {
        let next = transform_chain_item(state, prereqs, base_expression, item);
        transform_optional_chain_inner(state, prereqs, chain, next, temp_id, index + 1)
    }
}

/// `transformOptionalChain`
pub fn transform_optional_chain(state: &mut TransformState<'_>, prereqs: &mut Prereqs, node: P<Node>) -> Expr {
    let (chain, expression) = flatten_optional_chain(node);
    let base = transform_expression(state, prereqs, expression);
    transform_optional_chain_inner(state, prereqs, &chain, base, None, 0)
}
