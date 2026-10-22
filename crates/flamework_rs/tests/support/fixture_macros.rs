//! The transformer fixture's plugin (`packages/transformer/tests/fixture/fieldInfoPlugin.cjs`) as Rust macro types:
//! the test plugin of the F2 tests and of `examples/fw_build.rs` (DESIGN §11.8's worked example).

#![allow(dead_code)]

use flamework_rs::{Constituents, Flamework, Literal, MacroCx, MacroType, Primitive};
use rbxts_luau::{BinaryOperator, create};
use rbxts_plugin::ts::{P, Type};
use rbxts_plugin::{LuauExpr, PResult, text_literal};

/// `{ name, kind, optional, readonly }` for every field of an object type, each name after `prefix`.
pub struct FieldInfo {
    pub prefix: String,
}

impl MacroType for FieldInfo {
    fn expand(&self, cx: &mut MacroCx<'_, '_>, ty: P<Type>) -> PResult<LuauExpr> {
        let Some(fields) = cx.fields(ty) else { return Ok(create::array(Vec::new())) };
        let items = fields
            .into_iter()
            .map(|field| {
                let kind = describe(cx, field.ty);
                create::map(vec![
                    (create::string("name"), text_literal(&format!("{}{}", self.prefix, field.name))),
                    (create::string("kind"), text_literal(&kind)),
                    (create::string("optional"), create::bool(field.optional)),
                    (create::string("readonly"), create::bool(field.readonly)),
                ])
            })
            .collect();
        Ok(create::array(items))
    }
}

/// The version of the transformer running the plugin.
pub struct TransformerVersion;

impl MacroType for TransformerVersion {
    fn expand(&self, _: &mut MacroCx<'_, '_>, _: P<Type>) -> PResult<LuauExpr> {
        Ok(text_literal(flamework_rs::VERSION))
    }
}

/// The type, described as text.
pub struct Describe;

impl MacroType for Describe {
    fn expand(&self, cx: &mut MacroCx<'_, '_>, ty: P<Type>) -> PResult<LuauExpr> {
        let text = describe(cx, ty);
        Ok(text_literal(&text))
    }
}

/// A callable, `function(increment) return increment + 1 end`, with a warning naming the type.
pub struct Counter;

impl MacroType for Counter {
    fn expand(&self, cx: &mut MacroCx<'_, '_>, ty: P<Type>) -> PResult<LuauExpr> {
        let shown = cx.display(ty);
        cx.warning(format!("counter macro used with {shown}"));
        let body = create::binary(create::id("increment"), BinaryOperator::Add, create::number(1.0));
        Ok(create::function(vec![create::any_id("increment")], false, vec![create::return_stmt(body)]))
    }
}

/// The number of a union's members; an error at the call for anything else.
pub struct MustBeUnion;

impl MacroType for MustBeUnion {
    fn expand(&self, cx: &mut MacroCx<'_, '_>, ty: P<Type>) -> PResult<LuauExpr> {
        match cx.constituents(ty) {
            Some(Constituents::Union(members)) => Ok(create::number(members.len() as f64)),
            _ => Err(cx.error("mustBeUnion requires a union type")),
        }
    }
}

fn describe(cx: &mut MacroCx<'_, '_>, t: P<Type>) -> String {
    match cx.literal(t) {
        Some(Literal::Str(s)) => return format!("string:{s}"),
        Some(Literal::Num(n)) => return format!("number:{}", rbxts_luau::js_number_to_string(n)),
        Some(Literal::Bool(b)) => return format!("boolean:{b}"),
        None => {}
    }
    for (primitive, name) in
        [(Primitive::String, "string"), (Primitive::Number, "number"), (Primitive::Boolean, "boolean")]
    {
        if cx.is_primitive(t, primitive) {
            return name.to_owned();
        }
    }
    if let Some(elements) = cx.tuple_elements(t) {
        let inner: Vec<String> = elements.iter().map(|e| describe(cx, e.ty)).collect();
        return format!("tuple[{}]", inner.join(","));
    }
    if let Some(element) = cx.array_element(t) {
        return format!("array<{}>", describe(cx, element));
    }
    if let Some(Constituents::Union(members)) = cx.constituents(t) {
        let mut parts: Vec<String> = members.iter().map(|&m| describe(cx, m)).collect();
        parts.sort();
        return format!("union({})", parts.join("|"));
    }
    cx.display(t)
}

/// Flamework with the fixture's macro types, with the fixture's plugin options (`prefix: "fx"`).
pub fn flamework_with_fixture_macros() -> Flamework {
    Flamework::new()
        .macro_type("fieldInfo", FieldInfo { prefix: "fx".into() })
        .macro_type("transformerVersion", TransformerVersion)
        .macro_type("describe", Describe)
        .macro_type("counter", Counter)
        .macro_type("mustBeUnion", MustBeUnion)
}
