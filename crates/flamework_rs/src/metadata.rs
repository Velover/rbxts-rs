//! JSDoc `@metadata` requests (the TS transformer's `classes/nodeMetadata.ts`).
//!
//! A declaration's metadata is the union of the words in its own `@metadata` tags, those of the declarations its
//! decorators resolve to, and those of what it implements: an interface (for a class) or the interface's member of
//! the same name (for a class member). `{@link X key}` entries resolve `X` to a symbol and a type under `key`.
//!
//! Metadata holds this checker's symbols and types, so it is cached per instance (per checker), never shared.

use std::collections::{HashMap, HashSet};

use rbxts_plugin::Cx;
use rbxts_plugin::ts::{Kind, Node, P, Symbol, Type};
use tsrs_ast::SymbolFlags;

/// The metadata requested for one node.
#[derive(Debug, Default)]
pub struct NodeMetadata {
    set: HashSet<String>,
    symbols: HashMap<String, Vec<P<Symbol>>>,
    types: HashMap<String, Vec<P<Type>>>,
    /// Where each linked type came from (the `{@link}` part), for "the constraint is defined here".
    type_traces: Vec<(P<Type>, P<Node>)>,
}

impl NodeMetadata {
    /// Parses the metadata of `node` (`new NodeMetadata(state, node)`).
    pub fn parse(cx: &mut Cx<'_>, node: P<Node>) -> NodeMetadata {
        let mut metadata = NodeMetadata::default();
        metadata.parse_node(cx, node, &mut Vec::new());
        metadata
    }

    /// `isRequested`: the name (or `*`) is listed, and `~name` is not.
    pub fn is_requested(&self, name: &str) -> bool {
        if self.set.contains(&format!("~{name}")) {
            return false;
        }
        self.set.contains(name) || self.set.contains("*")
    }

    /// The symbols linked under `key` (`{@link X key}`).
    pub fn symbols(&self, key: &str) -> Option<&[P<Symbol>]> {
        self.symbols.get(key).map(Vec::as_slice)
    }

    /// The types linked under `key`.
    pub fn types(&self, key: &str) -> Option<&[P<Type>]> {
        self.types.get(key).map(Vec::as_slice)
    }

    /// The `{@link}` a linked type came from.
    pub fn type_trace(&self, ty: P<Type>) -> Option<P<Node>> {
        self.type_traces.iter().find(|(t, _)| *t == ty).map(|(_, n)| *n)
    }

    /// `parseText`: `text.trim().replace(/\s+/, " ").split(" ")`. Only the first run of whitespace is collapsed, as in
    /// the TS: later runs give empty words, which match nothing.
    fn parse_text(&mut self, text: &str) {
        let trimmed = crate::js::trim(text);
        let collapsed = match trimmed.find(char::is_whitespace) {
            Some(start) => {
                let end = trimmed[start..]
                    .find(|c: char| !c.is_whitespace())
                    .map_or(trimmed.len(), |i| start + i);
                format!("{} {}", &trimmed[..start], &trimmed[end..])
            }
            None => trimmed.to_owned(),
        };
        for word in collapsed.split(' ') {
            self.set.insert(word.to_owned());
        }
    }

    /// `parseMetadata`: one `@metadata` tag.
    fn parse_tag(&mut self, cx: &mut Cx<'_>, tag: P<Node>, links: &[rbxts_plugin::JsDocLink]) {
        let parts = tag.comments();
        let has_link = parts
            .iter()
            .any(|p| matches!(p.kind(), Kind::JSDocLink | Kind::JSDocLinkCode | Kind::JSDocLinkPlain));
        if !has_link {
            // A comment without links is one string in the TS (`typeof tag.comment === "string"`).
            let text: String = parts
                .iter()
                .filter(|p| p.kind() == Kind::JSDocText)
                .map(|p| p.as_jsdoc_text().text().concat())
                .collect();
            self.parse_text(&text);
            return;
        }
        let mut link_index = 0;
        for &part in parts {
            match part.kind() {
                Kind::JSDocText => self.parse_text(&part.as_jsdoc_text().text().concat()),
                Kind::JSDocLink | Kind::JSDocLinkCode | Kind::JSDocLinkPlain => {
                    let (name, text) = match part.kind() {
                        Kind::JSDocLink => {
                            let l = part.as_jsdoc_link();
                            (l.name(), l.text().concat())
                        }
                        Kind::JSDocLinkCode => {
                            let l = part.as_jsdoc_link_code();
                            (l.name(), l.text().concat())
                        }
                        _ => {
                            let l = part.as_jsdoc_link_plain();
                            (l.name(), l.text().concat())
                        }
                    };
                    let Some(name) = name else { continue };
                    // The compiler resolved the links of the tag in order (only those with a name).
                    let resolved = links.get(link_index).filter(|l| l.name == name).and_then(|l| l.symbol);
                    link_index += 1;
                    let Some(symbol) = resolved else { continue };
                    let symbol = cx.checker().skip_alias(symbol);
                    let ty = if symbol.flags().contains(SymbolFlags::TypeAlias) {
                        cx.checker().get_declared_type_of_symbol_exported(symbol)
                    } else {
                        cx.checker().get_type_at_location(name)
                    };
                    let key = text.trim().to_owned();
                    self.symbols.entry(key.clone()).or_default().push(symbol);
                    self.types.entry(key).or_default().push(ty);
                    self.type_traces.push((ty, part));
                }
                _ => {}
            }
        }
    }

    /// `parse`: the node's own tags, its decorators' declarations, and what it implements. `seen` stops a cycle of
    /// declarations (a decorator declared with itself as a decorator), which the TS would recurse on forever.
    fn parse_node(&mut self, cx: &mut Cx<'_>, node: P<Node>, seen: &mut Vec<P<Node>>) {
        if seen.contains(&node) {
            return;
        }
        seen.push(node);

        let tags = cx.jsdoc_tags(node);
        for tag in tags.iter() {
            if tag.name == "metadata" {
                self.parse_tag(cx, tag.node, &tag.links);
            }
        }

        if tsrs_ast::can_have_decorators(node) {
            for decorator in node.decorators() {
                let Some(expression) = decorator.expression() else { continue };
                let target = if expression.kind() == Kind::CallExpression {
                    expression.expression().unwrap_or(expression)
                } else {
                    expression
                };
                let Some(symbol) = get_symbol(cx, target) else { continue };
                for &declaration in symbol.declarations() {
                    self.parse_node(cx, declaration, seen);
                }
            }
        }

        if tsrs_ast::is_class_element(node) && let Some(name) = node.name() {
            // Interfaces can request metadata for their own properties and methods.
            let name = tsrs_ast::get_property_name_for_property_name_node(name);
            if let Some(owner) = node.parent()
                && tsrs_ast::is_class_like(owner)
                && name != tsrs_ast::INTERNAL_SYMBOL_NAME_MISSING
            {
                for implement in implements_names(owner) {
                    let Some(symbol) = get_symbol(cx, implement) else { continue };
                    let Some(member) = symbol.members().and_then(|m| m.lookup(name.as_ref())) else { continue };
                    for &declaration in member.declarations() {
                        self.parse_node(cx, declaration, seen);
                    }
                }
            }
        } else if tsrs_ast::is_class_like(node) {
            // Interfaces can request metadata for the classes that implement them.
            for implement in implements_names(node) {
                let Some(symbol) = get_symbol(cx, implement) else { continue };
                if let Some(&declaration) = symbol.declarations().first() {
                    self.parse_node(cx, declaration, seen);
                }
            }
        }
    }
}

/// `state.getSymbol(node)`: a named declaration's name's symbol, aliases skipped.
pub fn get_symbol(cx: &mut Cx<'_>, node: P<Node>) -> Option<P<Symbol>> {
    let node = if is_named_declaration(node) { node.name().unwrap_or(node) } else { node };
    let symbol = cx.checker().get_symbol_at_location_exported(node)?;
    Some(cx.checker().skip_alias(symbol))
}

/// `ts.isNamedDeclaration` restricted to declarations: the kinds a `state.getSymbol` caller passes.
fn is_named_declaration(node: P<Node>) -> bool {
    matches!(
        node.kind(),
        Kind::ClassDeclaration
            | Kind::ClassExpression
            | Kind::InterfaceDeclaration
            | Kind::TypeAliasDeclaration
            | Kind::FunctionDeclaration
            | Kind::MethodDeclaration
            | Kind::MethodSignature
            | Kind::PropertyDeclaration
            | Kind::PropertySignature
            | Kind::GetAccessor
            | Kind::SetAccessor
            | Kind::EnumDeclaration
            | Kind::ModuleDeclaration
            | Kind::VariableDeclaration
            | Kind::Parameter
    ) && node.name().is_some()
}

/// The names of a class's `implements` entries (`getEffectiveImplementsTypeNodes(..).expression`). TS7 makes each
/// entry a TypeReference: its type name (DESIGN §10.2).
pub fn implements_names(class: P<Node>) -> Vec<P<Node>> {
    implements_entries(class)
        .into_iter()
        .filter_map(|entry| match entry.kind() {
            Kind::TypeReference => Some(entry.as_type_reference_node().type_name()),
            Kind::ExpressionWithTypeArguments => entry.expression(),
            _ => None,
        })
        .collect()
}

/// The entries of a class's `implements` clauses, in order.
pub fn implements_entries(class: P<Node>) -> Vec<P<Node>> {
    let mut out = Vec::new();
    let Some(clauses) = class.class_like_data().and_then(|d| d.heritage_clauses()) else {
        return out;
    };
    for &clause in clauses.nodes() {
        let heritage = clause.as_heritage_clause();
        if heritage.token == Kind::ImplementsKeyword {
            out.extend(heritage.types.get().nodes().iter().copied());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::NodeMetadata;

    #[test]
    fn words_split_as_the_ts_transformer_splits_them() {
        let mut metadata = NodeMetadata::default();
        // `text.trim().replace(/\s+/, " ").split(" ")`: only the first run of whitespace becomes one space.
        metadata.parse_text("  reflect   identifier  flamework:parameters\tinjectable ");
        assert!(metadata.is_requested("reflect"));
        assert!(metadata.is_requested("identifier"));
        assert!(!metadata.is_requested("flamework:parameters"));
        assert!(metadata.is_requested("flamework:parameters\tinjectable"));
        // `~name` turns a name off, even under `*`.
        metadata.parse_text("* ~flamework:module");
        assert!(metadata.is_requested("anything"));
        assert!(!metadata.is_requested("flamework:module"));
    }
}
