//! Type and declaration ids (the TS transformer's `util/uid.ts`): `getTypeUid`, `getSymbolUid`,
//! `findValidDeclaration`, `getDeclarationUid`, on top of [`crate::ids`].
//!
//! An id is known at once ([`IdAnswer::Now`]: full mode, ids in a build info, package ids) or late
//! ([`IdAnswer::Late`]: `short` / `tiny` / `obfuscated`, assigned in `finish` in program order). Each request of a new
//! id is recorded with its place in the TS transformer's walk ([`crate::order`]); `end_file` keeps the first of each.

use rbxts_plugin::ts::{Kind, Node, P, Symbol, Type};
use rbxts_plugin::{Cx, LuauExpr, PResult, PluginDiagnostic, text_literal};
use tsrs_checker::TypeFlags;

use crate::ids::{self, Declaration, IdAnswer};
use crate::node_path;
use crate::plugin::FwInstance;

impl FwInstance {
    /// The Luau value of an id: a string, or a late string resolved in `finish`.
    pub(crate) fn id_expr(&self, cx: &mut Cx<'_>, id: IdAnswer) -> LuauExpr {
        match id {
            IdAnswer::Now(id) => text_literal(&id),
            IdAnswer::Late(key) => {
                let mut file = self.file.borrow_mut();
                if file.late.insert(key.clone()) {
                    file.part.late_keys.push(key.clone());
                }
                drop(file);
                cx.late_string(&key)
            }
        }
    }

    /// `getNodeTypeUid`: the id of the type at a node; an error at the node when it has none.
    pub(crate) fn node_type_uid(&self, cx: &mut Cx<'_>, node: P<Node>) -> PResult<IdAnswer> {
        let ty = cx.checker().get_type_at_location(node);
        self.type_uid(cx, ty, node)
    }

    /// `getTypeUid(state, type, trace)`: the id of a type; an error at `trace` when it has none.
    pub(crate) fn type_uid(&self, cx: &mut Cx<'_>, ty: P<Type>, trace: P<Node>) -> PResult<IdAnswer> {
        match self.type_uid_inner(cx, ty, Some(trace))? {
            Some(id) => Ok(id),
            // Every path without an id reported an error at `trace` already.
            None => {
                let shown = self.display(cx, ty);
                Err(cx.error(trace, format!("Could not find UID for type \"{shown}\"")))
            }
        }
    }

    fn display(&self, cx: &mut Cx<'_>, ty: P<Type>) -> String {
        cx.checker().type_to_string_exported(ty)
    }

    fn type_uid_inner(&self, cx: &mut Cx<'_>, ty: P<Type>, trace: Option<P<Node>>) -> PResult<Option<IdAnswer>> {
        let injectable = cx.checker().get_type_of_property_of_type_exported(ty, "_flamework_injectable");
        if let Some(config) = injectable {
            if let Some(id_override) = cx.checker().get_type_of_property_of_type_exported(config, "id") {
                return self.type_uid_inner(cx, id_override, trace);
            }
            let original = cx.checker().get_type_of_property_of_type_exported(config, "type");
            return match original {
                Some(original) if original != ty => self.type_uid_inner(cx, original, trace),
                _ => match trace {
                    Some(trace) => Err(cx.error(trace, "malformed injectable type")),
                    None => Ok(None),
                },
            };
        }

        // A type parameter cannot be resolved here; the placeholder keeps the parameter count and fails at
        // resolution time with the parameter's name.
        if ty.flags().intersects(TypeFlags::TypeParameter) {
            return Ok(Some(IdAnswer::Now(format!("$tp:{}", self.display(cx, ty)))));
        }

        if let Some(symbol) = ty.symbol() {
            return self.symbol_uid(cx, symbol, trace);
        }
        if is_defined_type(cx, ty) {
            return Ok(Some(IdAnswer::Now("$p:defined".into())));
        }
        if ty.flags().intersects(TypeFlags::Intrinsic) {
            return Ok(Some(IdAnswer::Now(format!("$p:{}", ty.as_intrinsic_type().intrinsic_name()))));
        }
        match trace {
            Some(trace) => {
                let shown = self.display(cx, ty);
                Err(cx.error(trace, format!("Could not find UID for type \"{shown}\"")))
            }
            None => Ok(None),
        }
    }

    /// `getSymbolUid`.
    fn symbol_uid(&self, cx: &mut Cx<'_>, symbol: P<Symbol>, trace: Option<P<Node>>) -> PResult<Option<IdAnswer>> {
        if symbol.declarations().is_empty() {
            return match trace {
                Some(trace) => {
                    Err(cx.error(trace, format!("This symbol does not have any ID: \"{}\"", symbol.name())))
                }
                None => Ok(None),
            };
        }
        if let Some(declaration) = find_valid_declaration(cx, symbol, trace)? {
            return Ok(Some(self.declaration_uid(cx, declaration)));
        }
        match trace {
            Some(trace) => {
                let diagnostic = PluginDiagnostic::error(
                    trace,
                    format!("Could not find a declaration with a unique ID for '{}'.", symbol.name()),
                )
                .with_hint(
                    "Only classes, interfaces, type aliases and functions have IDs; a type parameter or an anonymous type cannot be used here.",
                );
                Err(cx.report(diagnostic).expect("an error"))
            }
            None => Ok(None),
        }
    }

    /// `getDeclarationUid`: the id of a declaration, recording a new one in the file part.
    pub(crate) fn declaration_uid(&self, cx: &mut Cx<'_>, node: P<Node>) -> IdAnswer {
        let declaration = self.describe_declaration(cx, node);
        let (answer, new) = self.shared.ids.request(&self.shared.build_info, &declaration);
        if let Some(new) = new {
            let mut file = self.file.borrow_mut();
            let key = file.current_key.clone();
            let seq = file.next();
            file.id_requests.push((key, seq, new));
        }
        answer
    }

    /// `getInternalId` and the rest of a declaration's description.
    fn describe_declaration(&self, cx: &mut Cx<'_>, node: P<Node>) -> Declaration {
        let shared = &self.shared;
        let file = tsrs_ast::get_source_file_of_node(node).expect("a declaration is in a file");
        let file_name = file.get().file_name().to_owned();
        let name = declaration_name(cx, node);
        let package = cx.project().packages.package_of(&file_name);
        let package_name = package.as_ref().and_then(|p| p.name.clone()).unwrap_or_default();
        let roots: Vec<&str> = shared.root_dirs.iter().map(String::as_str).collect();
        let (internal_id, is_package) = if ids::is_project_file(&file_name, &roots) {
            let output = shared.path_translator.get_output_path(&file_name);
            (ids::project_internal_id(&package_name, &shared.project_dir, &output, &name), false)
        } else {
            let directory = package.as_ref().map(|p| p.dir.clone()).unwrap_or_default();
            (ids::package_internal_id(&package_name, &directory, &file_name, &name), true)
        };
        Declaration {
            internal_id,
            is_package,
            name,
            source_file: file_name.clone(),
            file_name: node_path::parse_name(&file_name),
        }
    }
}

/// `getDeclarationName`: the name with every named ancestor's before it (`Enum.Material.Air`).
pub(crate) fn declaration_name(cx: &Cx<'_>, node: P<Node>) -> String {
    let own = node.name().filter(|n| n.kind() == Kind::Identifier).map(|n| n.text());
    let mut ancestors: Vec<&'static str> = Vec::new();
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() != Kind::SourceFile
            && let Some(name) = p.name()
        {
            ancestors.push(cx.source_text(name));
        }
        parent = p.parent();
    }
    ancestors.reverse();
    ids::declaration_name(own, ancestors)
}

/// `findValidDeclaration`: the declaration that carries a symbol's id. More than one candidate (and not exactly one
/// class among them) is an error at `trace`, with each candidate as related information.
fn find_valid_declaration(
    cx: &mut Cx<'_>,
    symbol: P<Symbol>,
    trace: Option<P<Node>>,
) -> PResult<Option<P<Node>>> {
    let mut usable: Vec<P<Node>> = Vec::new();
    for &declaration in symbol.declarations() {
        if matches!(declaration.kind(), Kind::TypeLiteral | Kind::TypeAliasDeclaration) {
            let mut current = Some(declaration);
            while let Some(node) = current {
                if node.kind() == Kind::TypeAliasDeclaration {
                    break;
                }
                current = node.parent();
            }
            if let Some(alias) = current {
                usable.push(alias);
                continue;
            }
        }
        if matches!(
            declaration.kind(),
            Kind::InterfaceDeclaration | Kind::ClassDeclaration | Kind::FunctionDeclaration
        ) {
            usable.push(declaration);
        }
    }
    if usable.len() <= 1 {
        return Ok(usable.first().copied());
    }
    let classes: Vec<P<Node>> = usable.iter().copied().filter(|d| d.kind() == Kind::ClassDeclaration).collect();
    if classes.len() == 1 {
        return Ok(Some(classes[0]));
    }
    let Some(trace) = trace else { return Ok(None) };
    let mut diagnostic =
        PluginDiagnostic::error(trace, format!("There are multiple possible IDs for this symbol: {}", symbol.name()));
    for declaration in usable {
        diagnostic =
            diagnostic.with_related(declaration.name().unwrap_or(declaration), "This is a valid declaration with a unique ID.");
    }
    Err(cx.report(diagnostic).expect("an error"))
}

/// `isDefinedType` (roblox-ts's): an object type with no properties, signatures or index signatures.
fn is_defined_type(cx: &mut Cx<'_>, ty: P<Type>) -> bool {
    let checker = cx.checker();
    ty.flags() == TypeFlags::Object
        && checker.get_properties_of_type_exported(ty).is_empty()
        && checker.get_call_signatures(ty).is_empty()
        && checker.get_construct_signatures(ty).is_empty()
        && checker.get_number_index_type(ty).is_none()
        && checker.get_string_index_type(ty).is_none()
}
