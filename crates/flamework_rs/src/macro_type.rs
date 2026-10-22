//! User plugins: Flamework's macro types (DESIGN §11.8), the Rust side of `@flamework-experimental/transformer-plugin`.
//!
//! A macro type is called through a `Modding.Intrinsic<"plugin", [id, T], R>` parameter of a user macro. The calling
//! side in user code is the TS one; the plugin is a Rust value registered on [`crate::Flamework`] in the build's
//! `main`:
//!
//! ```ignore
//! flamework_rs::Flamework::new().macro_type("fieldInfo", my_macros::FieldInfo { prefix: String::new() })
//! ```
//!
//! [`MacroType::expand`] gets the type `T` and a [`MacroCx`] anchored at the macro call, and returns Luau. Flamework
//! caches the result per file by `(id, type)` and hoists a result that is not trivially duplicable (a literal or a
//! name) into a local before the first root statement that uses it, so the second use in a file reuses it
//! (`pluginHost.ts`'s `executeMacroType`).
//!
//! The TS API maps as the porting guide says (DESIGN §11.8): `api.options` are the value's fields, `api.version` is
//! [`crate::VERSION`], `ctx.hoist` / `hoistStatement` / `error` / `warning` / `fileName` are on [`MacroCx`], and
//! the `Type` facade's queries are [`MacroCx`] methods taking a `P<Type>`.

use std::sync::Arc;

use rbxts_plugin::ts::{Checker, Node, P, Type};
use rbxts_plugin::{Abort, Cx, LuauExpr, LuauStmt};
use tsrs_ast::SymbolFlags;
use tsrs_checker::{ElementFlags, LiteralValue, SignatureKind, TypeDataTag, TypeFlags};

/// A user plugin's macro type. Shared by every checker: never keep a `P<Type>` (or any other handle) in `self`.
pub trait MacroType: Send + Sync + 'static {
    /// The Luau value for the type `value`. Report problems with [`MacroCx::error`] (`return Err(cx.error(..))`).
    fn expand(&self, cx: &mut MacroCx<'_, '_>, value: P<Type>) -> rbxts_plugin::PResult<LuauExpr>;
}

/// The macro types a build registered, by id, in registration order.
#[derive(Clone, Default)]
pub struct MacroTypes {
    entries: Vec<(String, Arc<dyn MacroType>)>,
}

impl MacroTypes {
    /// Adds a macro type. Two with one id are refused, as `registerMacroType` refuses them.
    pub fn insert(&mut self, id: String, macro_type: Arc<dyn MacroType>) -> Result<(), String> {
        if self.get(&id).is_some() {
            return Err(format!("Flamework: the macro type '{id}' is registered twice"));
        }
        self.entries.push((id, macro_type));
        Ok(())
    }

    /// The macro type registered under `id`.
    pub fn get(&self, id: &str) -> Option<&Arc<dyn MacroType>> {
        self.entries.iter().find(|(name, _)| name == id).map(|(_, m)| m)
    }

    /// The registered ids, in registration order.
    pub fn ids(&self) -> Vec<&str> {
        self.entries.iter().map(|(name, _)| name.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl std::fmt::Debug for MacroTypes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.ids()).finish()
    }
}

/// A literal type's value ([`MacroCx::literal`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Str(String),
    Num(f64),
    Bool(bool),
}

/// The primitive names of the TS facade's `isPrimitive`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Any,
    Unknown,
    String,
    Number,
    BigInt,
    Boolean,
    True,
    False,
    Undefined,
    Void,
    Never,
}

impl Primitive {
    /// The intrinsic name TypeScript gives the type.
    pub fn name(self) -> &'static str {
        match self {
            Primitive::Any => "any",
            Primitive::Unknown => "unknown",
            Primitive::String => "string",
            Primitive::Number => "number",
            Primitive::BigInt => "bigint",
            Primitive::Boolean => "boolean",
            Primitive::True => "true",
            Primitive::False => "false",
            Primitive::Undefined => "undefined",
            Primitive::Void => "void",
            Primitive::Never => "never",
        }
    }
}

/// A union's or intersection's members ([`MacroCx::constituents`]), in TS7's fixed order.
#[derive(Clone, Debug, PartialEq)]
pub enum Constituents {
    Union(Vec<P<Type>>),
    Intersection(Vec<P<Type>>),
}

/// A property of an object type ([`MacroCx::fields`]), in declaration order.
#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub ty: P<Type>,
    pub optional: bool,
    pub readonly: bool,
}

/// An index signature ([`MacroCx::index_signatures`]).
#[derive(Clone, Debug)]
pub struct IndexSignature {
    pub key: P<Type>,
    pub value: P<Type>,
    pub readonly: bool,
}

/// A call or construct signature ([`MacroCx::call_signatures`]).
#[derive(Clone, Debug)]
pub struct SignatureInfo {
    pub inputs: Vec<P<Type>>,
    pub output: P<Type>,
}

/// A tuple element ([`MacroCx::tuple_elements`]).
#[derive(Clone, Debug)]
pub struct TupleElement {
    pub name: Option<String>,
    pub ty: P<Type>,
    pub spread: bool,
    pub optional: bool,
}

/// What a macro type can ask for: the call site, diagnostics, hoisting, and type queries on this checker. It wraps
/// the call hook's [`Cx`], anchored at the macro call.
pub struct MacroCx<'c, 'a> {
    cx: &'c mut Cx<'a>,
    node: P<Node>,
}

impl<'c, 'a> MacroCx<'c, 'a> {
    pub(crate) fn new(cx: &'c mut Cx<'a>, node: P<Node>) -> Self {
        MacroCx { cx, node }
    }

    /// The hook's context, for anything else (raw tsrs through `checker()`, which follows tsrs's names).
    pub fn cx(&mut self) -> &mut Cx<'a> {
        self.cx
    }

    /// The macro call (`ctx` diagnostics are reported here).
    pub fn node(&self) -> P<Node> {
        self.node
    }

    /// The file the call is in (`ctx.fileName`).
    pub fn file_name(&self) -> String {
        self.cx.file().path.clone()
    }

    /// This checker.
    pub fn checker(&mut self) -> &mut Checker {
        self.cx.checker()
    }

    /// `ctx.hoist(expr, name)`: a local holding `value`, declared before the current root statement, named after
    /// `name` as the TS plugin host names it (`name`, else `name_1`, …).
    pub fn hoist(&mut self, value: LuauExpr, name: &str) -> LuauExpr {
        crate::macros::hoist_named(self.cx, value, name)
    }

    /// `ctx.hoistStatement(stmt)`: statements before the current root statement.
    pub fn insert_before_root_statement(&mut self, stmts: impl IntoIterator<Item = LuauStmt>) {
        self.cx.insert_before_root_statement(stmts);
    }

    /// `ctx.error(msg)`: an error at the call; `return Err(cx.error("…"))`.
    pub fn error(&mut self, message: impl Into<String>) -> Abort {
        self.cx.error(self.node, message)
    }

    /// `ctx.warning(msg)`: a warning at the call.
    pub fn warning(&mut self, message: impl Into<String>) {
        self.cx.warning(self.node, message);
    }

    // --- the Type facade ---

    /// `isSubtypeOf`: `a` is assignable to `b`.
    pub fn is_subtype(&mut self, a: P<Type>, b: P<Type>) -> bool {
        self.checker().is_type_assignable_to_exported(a, b)
    }

    /// `isSupertypeOf`: `b` is assignable to `a`.
    pub fn is_supertype(&mut self, a: P<Type>, b: P<Type>) -> bool {
        self.checker().is_type_assignable_to_exported(b, a)
    }

    /// `isEquivalentTo`: assignable both ways.
    pub fn is_equivalent(&mut self, a: P<Type>, b: P<Type>) -> bool {
        self.is_subtype(a, b) && self.is_subtype(b, a)
    }

    /// `getConstituents`, for a union or an intersection.
    pub fn constituents(&mut self, t: P<Type>) -> Option<Constituents> {
        let flags = t.flags();
        if flags.intersects(TypeFlags::Union) {
            Some(Constituents::Union(t.as_union_or_intersection_type().types().to_vec()))
        } else if flags.intersects(TypeFlags::Intersection) {
            Some(Constituents::Intersection(t.as_union_or_intersection_type().types().to_vec()))
        } else {
            None
        }
    }

    /// `isObjectLike`.
    pub fn is_object_like(&self, t: P<Type>) -> bool {
        t.flags().intersects(TypeFlags::Object)
    }

    /// `getFields`: the properties of an object type, in declaration order; `None` when it is not object-like.
    pub fn fields(&mut self, t: P<Type>) -> Option<Vec<Field>> {
        if !self.is_object_like(t) {
            return None;
        }
        let mut fields = Vec::new();
        let properties = self.checker().get_properties_of_type_exported(t);
        for &property in properties {
            let name = property.name();
            let Some(ty) = self.checker().get_type_of_property_of_type_exported(t, name) else { continue };
            fields.push(Field {
                name: name.to_owned(),
                ty,
                optional: property.flags().intersects(SymbolFlags::Optional),
                readonly: property.declarations().iter().any(|&d| tsrs_checker::is_declaration_readonly(d)),
            });
        }
        Some(fields)
    }

    /// `getIndexSignatures`.
    pub fn index_signatures(&mut self, t: P<Type>) -> Vec<IndexSignature> {
        self.checker()
            .get_index_infos_of_type_exported(t)
            .iter()
            .map(|info| IndexSignature { key: info.key_type(), value: info.value_type(), readonly: info.is_readonly() })
            .collect()
    }

    /// `getCallSignatures`.
    pub fn call_signatures(&mut self, t: P<Type>) -> Vec<SignatureInfo> {
        self.signatures(t, SignatureKind::Call)
    }

    /// `getConstructSignatures`.
    pub fn construct_signatures(&mut self, t: P<Type>) -> Vec<SignatureInfo> {
        self.signatures(t, SignatureKind::Construct)
    }

    fn signatures(&mut self, t: P<Type>, kind: SignatureKind) -> Vec<SignatureInfo> {
        let signatures = self.checker().get_signatures_of_type(t, kind);
        signatures
            .iter()
            .map(|&signature| {
                let checker = self.checker();
                let inputs = signature.parameters().iter().map(|&p| checker.get_type_of_symbol(p)).collect();
                let output = checker.get_return_type_of_signature_exported(signature);
                SignatureInfo { inputs, output }
            })
            .collect()
    }

    /// `isArray`: an array or a tuple.
    pub fn is_array(&mut self, t: P<Type>) -> bool {
        self.checker().is_array_type(t) || tsrs_checker::is_tuple_type_exported(t)
    }

    /// `isTuple`.
    pub fn is_tuple(&self, t: P<Type>) -> bool {
        tsrs_checker::is_tuple_type_exported(t)
    }

    /// `getElementType`: an array's element type, or the union of a tuple's (`never` for `[]`); `None` otherwise.
    pub fn array_element(&mut self, t: P<Type>) -> Option<P<Type>> {
        if tsrs_checker::is_tuple_type_exported(t) {
            let elements = self.checker().get_type_arguments(t).to_vec();
            let checker = self.checker();
            return Some(if elements.is_empty() { checker.get_never_type() } else { checker.get_union_type(&elements) });
        }
        if !self.checker().is_array_type(t) {
            return None;
        }
        let checker = self.checker();
        Some(checker.get_element_type_of_array_type_exported(t).unwrap_or_else(|| checker.get_any_type()))
    }

    /// `isReadonly`: `readonly T[]` or `ReadonlyArray<T>` (`isReadonlyArrayType`: a reference to the global
    /// `ReadonlyArray`, which is what the TS facade's symbol test matches).
    pub fn is_readonly_array(&mut self, t: P<Type>) -> bool {
        self.checker().is_readonly_array_type(t)
    }

    /// `getElements`: a tuple's elements; `None` when it is not a tuple.
    pub fn tuple_elements(&mut self, t: P<Type>) -> Option<Vec<TupleElement>> {
        if !tsrs_checker::is_tuple_type_exported(t) {
            return None;
        }
        let infos = t.target().map(|target| target.as_tuple_type().element_infos()).unwrap_or_default();
        let arguments = self.checker().get_type_arguments(t);
        Some(
            arguments
                .iter()
                .enumerate()
                .map(|(index, &ty)| {
                    let info = infos.get(index);
                    let name = info
                        .and_then(|i| i.labeled_declaration)
                        .and_then(|d| d.name())
                        .filter(|n| n.kind() == tsrs_ast::Kind::Identifier)
                        .map(|n| n.text().to_owned());
                    let flags = info.map(|i| i.flags).unwrap_or(ElementFlags::Required);
                    TupleElement {
                        name,
                        ty,
                        spread: flags.intersects(ElementFlags::Rest),
                        optional: flags.intersects(ElementFlags::Optional),
                    }
                })
                .collect(),
        )
    }

    /// `isLiteral` + `getLiteralValue`: a string, number or boolean literal type's value.
    pub fn literal(&self, t: P<Type>) -> Option<Literal> {
        literal_of(t)
    }

    /// `isPrimitive(name)`.
    pub fn is_primitive(&self, t: P<Type>, primitive: Primitive) -> bool {
        intrinsic_name(t) == Some(primitive.name())
    }

    /// `getNonNullable`.
    pub fn non_nullable(&mut self, t: P<Type>) -> P<Type> {
        self.checker().get_non_nullable_type(t)
    }

    /// `isOptional`: a union with `undefined` or `null` in it.
    pub fn is_optional(&self, t: P<Type>) -> bool {
        t.flags().intersects(TypeFlags::Union)
            && t.as_union_or_intersection_type()
                .types()
                .iter()
                .any(|m| m.flags().intersects(TypeFlags::Undefined | TypeFlags::Null))
    }

    /// `getName`: the alias's name, else the symbol's.
    pub fn type_name(&self, t: P<Type>) -> Option<String> {
        if let Some(alias) = t.alias()
            && let Some(symbol) = alias.symbol.get()
        {
            return Some(symbol.name().to_owned());
        }
        t.symbol().map(|s| s.name().to_owned())
    }

    /// `toString`: `typeToString`.
    pub fn display(&mut self, t: P<Type>) -> String {
        self.checker().type_to_string_exported(t)
    }
}

/// A string, number or boolean literal type's value.
pub(crate) fn literal_of(t: P<Type>) -> Option<Literal> {
    if t.data_tag() != TypeDataTag::Literal {
        return None;
    }
    match t.as_literal_type().value()? {
        LiteralValue::String(s) => Some(Literal::Str(s.to_owned())),
        LiteralValue::Number(n) => Some(Literal::Num(n.0)),
        LiteralValue::Boolean(b) => Some(Literal::Bool(b)),
        LiteralValue::BigInt(_) => None,
    }
}

/// An intrinsic type's name (`string`, `undefined`, …), `boolean` for the boolean union, and `true` / `false` for the
/// boolean literals (TS5 made those intrinsic types; TS7 makes them literal types).
pub(crate) fn intrinsic_name(t: P<Type>) -> Option<&'static str> {
    let flags = t.flags();
    if flags.intersects(TypeFlags::Boolean) && flags.intersects(TypeFlags::Union) {
        return Some("boolean");
    }
    if flags.intersects(TypeFlags::BooleanLiteral) {
        return match literal_of(t) {
            Some(Literal::Bool(true)) => Some("true"),
            Some(Literal::Bool(false)) => Some("false"),
            _ => None,
        };
    }
    (t.data_tag() == TypeDataTag::Intrinsic).then(|| t.as_intrinsic_type().intrinsic_name())
}

/// `isTriviallyDuplicable`: a literal or a name is emitted at every use rather than hoisted.
pub(crate) fn is_trivially_duplicable(expression: &LuauExpr) -> bool {
    matches!(
        expression,
        LuauExpr::String(_)
            | LuauExpr::LateString(_)
            | LuauExpr::Number(_)
            | LuauExpr::True
            | LuauExpr::False
            | LuauExpr::Nil
            | LuauExpr::Identifier(_)
            | LuauExpr::TemporaryIdentifier(_)
    )
}

/// `executeMacroType`: the cached or new result of macro type `id` for `input` at the call `node`.
pub(crate) fn execute(
    fw: &crate::FwInstance,
    cx: &mut Cx<'_>,
    node: P<Node>,
    id: &str,
    input: P<Type>,
) -> rbxts_plugin::PResult<LuauExpr> {
    let key = (id.to_owned(), input);
    if let Some(cached) = fw.file.borrow().macro_cache.get(&key) {
        return Ok(cached.clone());
    }
    let Some(macro_type) = fw.macro_types.get(id).cloned() else {
        let registered = fw.macro_types.ids();
        let diagnostic = if registered.is_empty() {
            rbxts_plugin::PluginDiagnostic::error(
                node,
                format!("The macro type '{id}' requires a plugin, but no macro types are registered."),
            )
            .with_hint(format!(
                "Register it in the build's main.rs: flamework_rs::Flamework::new().macro_type(\"{id}\", …)."
            ))
        } else {
            rbxts_plugin::PluginDiagnostic::error(node, format!("No loaded plugin registered the macro type '{id}'."))
                .with_hint(format!("Registered macro types: {}", registered.join(", ")))
        };
        return Err(cx.report(diagnostic).expect("an error"));
    };
    let result = macro_type.expand(&mut MacroCx::new(cx, node), input)?;
    let expression = if is_trivially_duplicable(&result) { result } else { crate::macros::hoist_named(cx, result, id) };
    fw.file.borrow_mut().macro_cache.insert(key, expression.clone());
    Ok(expression)
}

