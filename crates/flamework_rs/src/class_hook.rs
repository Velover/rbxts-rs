//! Class metadata (the TS transformer's `transformations/statements/transformClassDeclaration.ts`, with
//! `getFlameworkDecorators.ts` and `validateConstraintMetadata.ts`).
//!
//! A class whose metadata (its own JSDoc, its decorators' declarations, what it implements, or any member's)
//! requests `reflect` gets `Reflect.defineMetadata(Class, key, value[, property])` calls in a `do` block at the
//! static-block position, after the class is created and before its decorators run: `@Provider()` and
//! `@Component()` read them. Keys: `identifier`, `flamework:module`, `flamework:implements`, and from the first
//! construct signature `flamework:parameters` / `parameter_names` / `dependencies`; per member `flamework:type` and
//! `flamework:return_type`, and the parameter keys of methods.
//!
//! Not yet: guards (`flamework:guard`, `flamework:parameter_guards`, `flamework:return_guard`, and a guard in a
//! `Modding.Injectable` dependency's metadata: F3). A class that requests them gets an error naming the milestone,
//! rather than metadata that silently lacks them.

use rbxts_luau::create;
use rbxts_plugin::ts::{Kind, Node, P, Type};
use rbxts_plugin::{ClassSite, Cx, LuauExpr, LuauStmt, PResult, PluginDiagnostic, text_literal};
use tsrs_ast::SymbolFlags;

use crate::macros::{Analysis, Builder, UserMacro};
use crate::metadata::{NodeMetadata, get_symbol, implements_entries};
use crate::packages::{COMPONENTS_PACKAGE, CORE_PACKAGE};
use crate::plugin::FwInstance;

/// Which of `@Provider()` and `@Component()` a class carries itself.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DecoratorKinds {
    pub provider: bool,
    pub component: bool,
}

/// A parameter with the type it has at the class being transformed (a constructor inherited from a generic base is
/// seen with its type arguments applied).
struct ParameterInfo {
    declaration: Option<P<Node>>,
    ty: P<Type>,
    trace: P<Node>,
}

type Fields = Vec<(&'static str, LuauExpr)>;

/// The class hook.
pub(crate) fn class_metadata(fw: &FwInstance, cx: &mut Cx<'_>, class: &ClassSite) -> PResult<Vec<LuauStmt>> {
    let node = class.node;
    // Class expressions stay unhandled, as in the TS transformer.
    if node.kind() != Kind::ClassDeclaration {
        return Ok(Vec::new());
    }
    let Some(class_name) = node.name() else { return Ok(Vec::new()) };
    if get_symbol(cx, node).is_none() {
        return Ok(Vec::new());
    }
    let metadata = fw.metadata(cx, node);
    if !has_reflect_metadata(fw, cx, node, &metadata) {
        return Ok(Vec::new());
    }

    // The TS transformer asks for these ids before it transforms the members.
    let key = fw.order_key(cx, node, crate::order::BEFORE_CHILDREN);
    fw.file.borrow_mut().current_key = key;

    let specifier = fw.shared.reflect_specifier(&cx.file().path);
    let reflect = cx.import_named(&specifier, "Reflect")?;
    let name_text = class_name.text();

    let mut statements: Vec<LuauStmt> = Vec::new();
    let fields = class_fields(fw, cx, node, &metadata)?;
    convert(&mut statements, &reflect, &class.luau_name, fields, None, name_text);
    validate_constraint_metadata(fw, cx, node, &metadata);

    for &member in node.members() {
        let Some(member_name) = member.name() else { continue };
        let property = tsrs_ast::get_property_name_for_property_name_node(member_name);
        if property == tsrs_ast::InternalSymbolNameMissing {
            continue;
        }
        let member_metadata = fw.metadata(cx, member);
        let reflection = match member.kind() {
            Kind::MethodDeclaration => method_fields(fw, cx, member, &member_metadata)?,
            Kind::PropertyDeclaration => field_fields(fw, cx, member, &member_metadata)?,
            _ => Vec::new(),
        };
        if !reflection.is_empty() && tsrs_ast::has_static_modifier(member) {
            return Err(cx.error(member_name, "Flamework does not support reflection on static members."));
        }
        convert(&mut statements, &reflect, &class.luau_name, reflection, Some(&property), name_text);
        validate_constraint_metadata(fw, cx, member, &member_metadata);
    }

    Ok(vec![create::do_stmt(statements)])
}

/// `convertReflectionToStatements` with its section comment.
fn convert(
    out: &mut Vec<LuauStmt>,
    reflect: &LuauExpr,
    class: &LuauExpr,
    fields: Fields,
    property: Option<&str>,
    class_name: &str,
) {
    if fields.is_empty() {
        return;
    }
    let element = match property {
        Some(property) => format!("{class_name}.{property}"),
        None => class_name.to_owned(),
    };
    out.push(create::comment(format!(" (Flamework) {element} metadata")));
    for (key, value) in fields {
        let mut args = vec![class.clone(), text_literal(key), value];
        if let Some(property) = property {
            args.push(text_literal(property));
        }
        out.push(create::call_statement(create::call(create::property(reflect.clone(), "defineMetadata"), args)));
    }
}

pub(crate) fn has_reflect_metadata(fw: &FwInstance, cx: &mut Cx<'_>, class: P<Node>, metadata: &NodeMetadata) -> bool {
    if metadata.is_requested("reflect") {
        return true;
    }
    class.members().iter().any(|&member| fw.metadata(cx, member).is_requested("reflect"))
}

/// `generateClassMetadata`.
fn class_fields(fw: &FwInstance, cx: &mut Cx<'_>, node: P<Node>, metadata: &NodeMetadata) -> PResult<Fields> {
    let mut fields: Fields = Vec::new();

    if metadata.is_requested("identifier") {
        let id = fw.node_type_uid(cx, node)?;
        fields.push(("identifier", fw.id_expr(cx, id)));
        // The module that defined the class, for path registration: only a class its module creates once.
        if is_module_level(node) {
            fields.push(("flamework:module", create::id("script")));
        }
    }

    if metadata.is_requested("flamework:implements") {
        let mut values = Vec::new();
        for entry in implements_entries(node) {
            let id = fw.node_type_uid(cx, entry)?;
            values.push(fw.id_expr(cx, id));
        }
        fields.push(("flamework:implements", create::array(values)));
    }

    let symbol = crate::metadata::get_symbol(cx, node).expect("checked by the hook");
    let constructor_type = cx.checker().get_type_of_symbol(symbol);
    let first = cx.checker().get_construct_signatures(constructor_type).first().copied();
    if let Some(signature) = first {
        let trace_default = node.name().unwrap_or(node);
        let mut parameters = Vec::new();
        for (index, &parameter) in signature.parameters().iter().enumerate() {
            let declaration = parameter.declarations().iter().copied().find(|d| d.kind() == Kind::Parameter);
            let ty = cx.checker().get_type_at_position_exported(signature, index);
            parameters.push(ParameterInfo { declaration, ty, trace: declaration.unwrap_or(trace_default) });
        }
        fields.extend(parameter_fields(fw, cx, metadata, &parameters)?);

        if metadata.is_requested("flamework:dependencies") && fw.decorator_kinds(cx, node).provider {
            validate_provider_parameters(fw, cx, node, &parameters)?;
        }
    }

    Ok(fields)
}

/// `generateParametersMetadata`.
fn parameter_fields(
    fw: &FwInstance,
    cx: &mut Cx<'_>,
    metadata: &NodeMetadata,
    parameters: &[ParameterInfo],
) -> PResult<Fields> {
    let mut fields: Fields = Vec::new();

    if metadata.is_requested("flamework:parameters") {
        let mut values = Vec::new();
        for parameter in parameters {
            let id = fw.type_uid(cx, parameter.ty, parameter.trace)?;
            values.push(fw.id_expr(cx, id));
        }
        fields.push(("flamework:parameters", create::array(values)));
    }

    if metadata.is_requested("flamework:parameter_names") {
        let values = parameters
            .iter()
            .map(|parameter| {
                let name = parameter
                    .declaration
                    .and_then(|d| d.name())
                    .filter(|n| n.kind() == Kind::Identifier)
                    .map_or("_binding_", |n| n.text());
                text_literal(name)
            })
            .collect();
        fields.push(("flamework:parameter_names", create::array(values)));
    }

    if metadata.is_requested("flamework:parameter_guards") {
        let mut values = Vec::new();
        for parameter in parameters {
            let at = parameter.declaration.unwrap_or(parameter.trace);
            values.push(crate::guards::guard_for(cx, fw.shared.guard_config(), at, parameter.ty)?);
        }
        fields.push(("flamework:parameter_guards", create::array(values)));
    }

    if metadata.is_requested("flamework:dependencies") {
        let mut values = Vec::new();
        for parameter in parameters {
            values.push(dependency_metadata(fw, cx, parameter.trace, parameter.ty)?);
        }
        fields.push(("flamework:dependencies", create::array(values)));
    }

    Ok(fields)
}

/// `getDependencyInjectionMetadata(state, node, type)` (not concise): `{ id = <id>, metadata = {...}? }`, the
/// metadata being the user macros of a `Modding.Injectable<{ metadata: [...] }>` dependency.
fn dependency_metadata(fw: &FwInstance, cx: &mut Cx<'_>, trace: P<Node>, ty: P<Type>) -> PResult<LuauExpr> {
    let metadata = Analysis { fw, node: trace }.injectable_metadata(cx, ty)?;
    if let Some(later) = metadata.as_ref().and_then(|items| items.iter().find_map(UserMacro::later)) {
        let what = format!("the metadata of a Modding.Injectable dependency ({})", later.what);
        let feature = match later.milestone {
            "F3" => "guard generation",
            "F4" => "the serializer",
            "F5" => "networking",
            _ => "components",
        };
        return Err(unported_error(cx, trace, &what, feature, later.milestone));
    }
    let key = fw.file.borrow().current_key.clone();
    Builder { fw, node: trace, key }.dependency(cx, ty, false, metadata.as_deref())
}

/// `generateFieldMetadata`.
fn field_fields(fw: &FwInstance, cx: &mut Cx<'_>, field: P<Node>, metadata: &NodeMetadata) -> PResult<Fields> {
    let mut fields: Fields = Vec::new();
    let ty = cx.checker().get_type_at_location(field);
    if metadata.is_requested("flamework:type") {
        let id = fw.type_uid(cx, ty, field.name().unwrap_or(field))?;
        fields.push(("flamework:type", fw.id_expr(cx, id)));
    }
    if metadata.is_requested("flamework:guard") {
        let at = field.type_node().unwrap_or(field);
        fields.push(("flamework:guard", crate::guards::guard_for(cx, fw.shared.guard_config(), at, ty)?));
    }
    Ok(fields)
}

/// `generateMethodMetadata`.
fn method_fields(fw: &FwInstance, cx: &mut Cx<'_>, method: P<Node>, metadata: &NodeMetadata) -> PResult<Fields> {
    let mut parameters = Vec::new();
    for &declaration in method.parameters() {
        let ty = cx.checker().get_type_at_location(declaration);
        parameters.push(ParameterInfo { declaration: Some(declaration), ty, trace: declaration });
    }
    let mut fields = parameter_fields(fw, cx, metadata, &parameters)?;

    let signature = cx.checker().get_signature_from_declaration_exported(method);
    if metadata.is_requested("flamework:return_type") {
        let return_type = cx.checker().get_return_type_of_signature_exported(signature);
        let id = fw.type_uid(cx, return_type, method.name().unwrap_or(method))?;
        fields.push(("flamework:return_type", fw.id_expr(cx, id)));
    }
    if metadata.is_requested("flamework:return_guard") {
        let at = method.type_node().unwrap_or(method);
        let return_type = cx.checker().get_return_type_of_signature_exported(signature);
        fields.push(("flamework:return_guard", crate::guards::guard_for(cx, fw.shared.guard_config(), at, return_type)?));
    }
    Ok(fields)
}

/// The error for metadata a later milestone brings.
fn unported_error(
    cx: &mut Cx<'_>,
    node: P<Node>,
    what: &str,
    feature: &str,
    milestone: &str,
) -> rbxts_plugin::Abort {
    cx.error(
        node,
        format!("{what} needs {feature}, which rbxts-rs's Flamework does not have yet ({milestone})."),
    )
}

/// `isModuleLevel`: declared at the top level of the file, or of a namespace in it.
fn is_module_level(node: P<Node>) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        match p.kind() {
            Kind::SourceFile => return true,
            Kind::ModuleBlock | Kind::ModuleDeclaration => parent = p.parent(),
            _ => return false,
        }
    }
    false
}

impl FwInstance {
    /// `getFlameworkDecorators`: the decorators a class carries itself, recognised by the function of that name
    /// exported by the Flamework package that declares it.
    pub(crate) fn decorator_kinds(&self, cx: &mut Cx<'_>, class: P<Node>) -> DecoratorKinds {
        if let Some(&kinds) = self.decorators.borrow().get(&class) {
            return kinds;
        }
        let mut kinds = DecoratorKinds::default();
        if tsrs_ast::can_have_decorators(class) {
            for decorator in class.decorators() {
                let Some(expression) = decorator.expression() else { continue };
                let target = if expression.kind() == Kind::CallExpression {
                    expression.expression().unwrap_or(expression)
                } else {
                    expression
                };
                let Some(symbol) = get_symbol(cx, target) else { continue };
                for &declaration in symbol.declarations() {
                    let package = declaring_package(cx, declaration);
                    match (symbol.name(), package.as_deref()) {
                        ("Provider", Some(CORE_PACKAGE)) => kinds.provider = true,
                        ("Component", Some(COMPONENTS_PACKAGE)) => kinds.component = true,
                        _ => {}
                    }
                }
            }
        }
        self.decorators.borrow_mut().insert(class, kinds);
        kinds
    }

    /// `getComponentOnlyClass`: the class declaration a type is the instance type of, when the class is a
    /// `@Component()` and not a `@Provider()`.
    pub(crate) fn component_only_class(&self, cx: &mut Cx<'_>, ty: P<Type>) -> Option<P<Node>> {
        let symbol = ty.symbol()?;
        if !symbol.flags().intersects(SymbolFlags::Class) {
            return None;
        }
        for &declaration in symbol.declarations() {
            if !matches!(declaration.kind(), Kind::ClassDeclaration | Kind::ClassExpression) {
                continue;
            }
            let kinds = self.decorator_kinds(cx, declaration);
            if kinds.component && !kinds.provider {
                return Some(declaration);
            }
        }
        None
    }
}

/// `getDeclaringPackage`: the name of the package whose package.json is nearest above the declaring file.
pub(crate) fn declaring_package(cx: &Cx<'_>, declaration: P<Node>) -> Option<String> {
    let file = tsrs_ast::get_source_file_of_node(declaration)?;
    cx.project().packages.package_of(file.get().file_name())?.name.clone()
}

/// `validateProviderParameters`: a `@Provider()` cannot take a component in its constructor.
fn validate_provider_parameters(
    fw: &FwInstance,
    cx: &mut Cx<'_>,
    node: P<Node>,
    parameters: &[ParameterInfo],
) -> PResult<()> {
    for parameter in parameters {
        let Some(component) = fw.component_only_class(cx, parameter.ty) else { continue };
        let component_name = match component.name() {
            Some(name) => name.text().to_owned(),
            None => cx.checker().type_to_string_exported(parameter.ty),
        };
        let provider_name = node.name().map_or("this provider", |n| n.text());
        let diagnostic = PluginDiagnostic::error(
            parameter.trace,
            format!(
                "'{provider_name}' takes '{component_name}' in its constructor, but '{component_name}' is a component (@Component), not a provider: a module only injects providers, and never constructs a component."
            ),
        )
        .with_hint(format!(
            "Make '{component_name}' a @Provider(), or take Components and get the component from the instance it is attached to: components.getComponent<{component_name}>(instance)."
        ));
        return Err(cx.report(diagnostic).expect("an error"));
    }
    Ok(())
}

/// `validateConstraintMetadata`: the node's type must be assignable to every `{@link T constraint}` type. Reported
/// without stopping, as in the TS.
fn validate_constraint_metadata(fw: &FwInstance, cx: &mut Cx<'_>, node: P<Node>, metadata: &NodeMetadata) {
    let _ = fw;
    let Some(name) = node.name() else { return };
    let Some(constraints) = metadata.types("constraint") else { return };
    let constraints = constraints.to_vec();
    let Some(symbol) = get_symbol(cx, name) else { return };
    let Some(node_type) = cx.checker().get_type_of_symbol_at_location(symbol, Some(node)) else { return };
    for constraint in constraints {
        if cx.checker().is_type_assignable_to_exported(node_type, constraint) {
            continue;
        }
        let source = cx.checker().type_to_string_exported(node_type);
        let target = cx.checker().type_to_string_exported(constraint);
        let mut diagnostic =
            PluginDiagnostic::error(name, format!("Type '{source}' does not satify constraint '{target}'"));
        if let Some(trace) = metadata.type_trace(constraint) {
            diagnostic = diagnostic.with_related(trace, "The constraint is defined here.");
        }
        cx.report(diagnostic);
    }
}
