//! User macros (the TS transformer's `transformations/transformUserMacro.ts`), as far as F1 goes: the `path` and
//! `pathglob` intrinsics that `registerProviders`, `registerProvidersGlob`, `requireModules`, `getClassesInPath`,
//! `registerComponents` and the user's own path macros take (DESIGN §10.2; where the line is drawn below).
//!
//! A user macro is a call whose callee's declaration has `@metadata macro`. Its marker parameters whose argument is
//! missing or `undefined` get generated metadata, missing ones before them are padded with `nil`, and the call is
//! otherwise compiled by roblox-ts as written (`compile_call` with argument slots).
//!
//! **The F1/F2 line.** F1 takes a macro call only when every generated parameter is a `path` or `pathglob`
//! intrinsic and the declaration carries no parameter intrinsic (`intrinsic-inline`, `-flamework-rewrite`,
//! `-const`, `-middleware`, `-component-config`, `-serializer-options`). Everything else (`Generic` ids and
//! dependencies, `Caller`, hashes, labels, `Many`, `Constant`, the other intrinsics) is F2 or later: the call is
//! declined before anything is compiled, so roblox-ts compiles it as a plain call, and the site is recorded for one
//! warning per build. The decision reads only the AST and types (the seam's rule: a hook cannot decline after it
//! compiled something).

use rbxts_luau::create;
use rbxts_plugin::ts::{Kind, Node, P, Type};
use rbxts_plugin::{Arg, CallKind, CallSite, Callee, Claim, Cx, LuauExpr, PResult, PluginDiagnostic, text_literal};
use tsrs_checker::{ElementFlags, LiteralValue, TypeFlags};

use crate::build_info::{GlobUse, PathUse};
use crate::node_path;
use crate::packages::CORE_PACKAGE;
use crate::order::OrderKey;
use crate::plugin::{FwInstance, PathEvent};

/// A marker parameter's macro, as far as F1 needs to tell them apart.
enum UserMacro {
    /// `Modding.Intrinsic<"path", [T]>`.
    Path(P<Type>),
    /// `Modding.Intrinsic<"pathglob", [T]>`.
    PathGlob(P<Type>),
    /// Anything else: what it is, and the milestone that brings it.
    Later(String, &'static str),
}

/// The intrinsics a macro declaration can carry on its parameters, with their milestone.
const PARAMETER_INTRINSICS: [(&str, &str); 6] = [
    ("intrinsic-inline", "F2"),
    ("intrinsic-flamework-rewrite", "F2"),
    ("intrinsic-const", "F2"),
    ("intrinsic-middleware", "F5"),
    ("intrinsic-component-config", "F6"),
    ("intrinsic-serializer-options", "F4"),
];

/// The call hook for a callee classified as a user macro.
pub(crate) fn user_macro(fw: &FwInstance, cx: &mut Cx<'_>, call: &CallSite) -> PResult<Claim<LuauExpr>> {
    // `super()` is skipped, as there is not enough information to evaluate it.
    if call.kind == CallKind::SuperCall {
        return Ok(Claim::Decline);
    }
    // The node the TS transformer sees: the call expression (a decorator's own call).
    let expression = match call.kind {
        CallKind::Decorator => match call.node.expression() {
            Some(e) if e.kind() == Kind::CallExpression => e,
            _ => return Ok(Claim::Decline),
        },
        _ => call.node,
    };

    let signature = cx.checker().get_resolved_signature_exported(expression);
    let Some(declaration) = signature.declaration() else { return Ok(Claim::Decline) };
    let metadata = fw.metadata(cx, declaration);

    let mut later: Option<(String, &'static str)> = PARAMETER_INTRINSICS
        .iter()
        .find(|(key, _)| metadata.symbols(key).is_some())
        .map(|(key, milestone)| (format!("the {key} parameter intrinsic"), *milestone));

    let mut parameters: Vec<(usize, UserMacro)> = Vec::new();
    if later.is_none() {
        for index in 0..parameter_count(cx, signature) {
            if !is_undefined_argument(call.args.get(index).copied()) {
                continue;
            }
            let parameter_type = cx.checker().get_type_at_position_exported(signature, index);
            let target = cx.checker().get_non_nullable_type(parameter_type);
            match macro_of_union(cx, target) {
                None => {}
                Some(UserMacro::Later(what, milestone)) => {
                    later = Some((what, milestone));
                    break;
                }
                Some(found) => parameters.push((index, found)),
            }
        }
    }

    if let Some((what, milestone)) = later {
        let callee = macro_call_shape(cx, expression, false).0;
        fw.unported(cx, expression, format!("{callee}: {what}"), milestone);
        return Ok(Claim::Decline);
    }
    if parameters.is_empty() {
        // Nothing to generate: the call as written, which is what roblox-ts compiles anyway.
        return Ok(Claim::Decline);
    }

    // Taken: from here on, build and compile.
    let highest = parameters.iter().map(|(i, _)| *i).max().unwrap_or(0);
    let count = call.args.len().max(highest + 1);
    let mut args = Vec::with_capacity(count);
    for index in 0..count {
        if let Some((_, found)) = parameters.iter().find(|(i, _)| *i == index) {
            let key = fw.order_key(cx, expression, (0, index as u32));
            let value = match found {
                UserMacro::Path(ty) => build_path(fw, cx, expression, *ty, key)?,
                UserMacro::PathGlob(ty) => build_path_glob(fw, cx, expression, *ty, key)?,
                UserMacro::Later(..) => unreachable!("declined above"),
            };
            args.push(Arg::Luau(value));
        } else {
            match call.args.get(index) {
                Some(&arg) => args.push(Arg::Original(arg)),
                None => args.push(Arg::Nil),
            }
        }
    }
    cx.compile_call(call, Callee::Original, args).map(Claim::Take)
}

/// `isUndefinedArgument`: missing, or the identifier `undefined`.
fn is_undefined_argument(argument: Option<P<Node>>) -> bool {
    match argument {
        None => true,
        Some(node) => node.kind() == Kind::Identifier && node.text() == "undefined",
    }
}

/// `getParameterCount`: a rest parameter typed as a tuple counts its fixed elements.
fn parameter_count(cx: &mut Cx<'_>, signature: P<tsrs_checker::Signature>) -> usize {
    let parameters = signature.parameters();
    let length = parameters.len();
    if signature.has_rest_parameter()
        && let Some(&last) = parameters.last()
    {
        let rest = cx.checker().get_type_of_symbol(last);
        if tsrs_checker::is_tuple_type_exported(rest)
            && let Some(target) = rest.target()
        {
            let tuple = target.as_tuple_type();
            let has_rest_element = tuple.combined_flags.get().intersects(ElementFlags::Variable);
            return length + tuple.fixed_length() - usize::from(!has_rest_element);
        }
    }
    length
}

/// `getUserMacroOfUnion`: the first constituent that is a macro.
fn macro_of_union(cx: &mut Cx<'_>, target: P<Type>) -> Option<UserMacro> {
    if target.flags().intersects(TypeFlags::Union) {
        for &constituent in target.as_union_or_intersection_type().types() {
            if let Some(found) = macro_of_type(cx, constituent) {
                return Some(found);
            }
        }
        return None;
    }
    macro_of_type(cx, target)
}

fn property(cx: &mut Cx<'_>, ty: P<Type>, name: &str) -> Option<P<Type>> {
    cx.checker().get_type_of_property_of_type_exported(ty, name)
}

/// `getUserMacroOfType` / `getBasicUserMacro`, telling only `path` and `pathglob` apart.
fn macro_of_type(cx: &mut Cx<'_>, target: P<Type>) -> Option<UserMacro> {
    if property(cx, target, "_flamework_macro_shared_ref").is_some() {
        return Some(UserMacro::Later("a Modding.Caller.Constant parameter".into(), "F2"));
    }
    if property(cx, target, "_flamework_macro_many").is_some() {
        return Some(UserMacro::Later("a Modding.Many parameter".into(), "F2"));
    }
    if let Some(generic) = property(cx, target, "_flamework_macro_generic") {
        let kind = property(cx, generic, "1").and_then(string_literal).unwrap_or_default();
        return Some(UserMacro::Later(format!("a Modding.Generic<T, \"{kind}\"> parameter"), "F2"));
    }
    if property(cx, target, "_flamework_macro_caller").is_some() {
        return Some(UserMacro::Later("a Modding.Caller parameter".into(), "F2"));
    }
    if property(cx, target, "_flamework_macro_hash").is_some() {
        return Some(UserMacro::Later("a Modding.Hash parameter".into(), "F2"));
    }
    let non_nullable = cx.checker().get_non_nullable_type(target);
    if property(cx, non_nullable, "_flamework_macro_tuple_labels").is_some() {
        return Some(UserMacro::Later("a Modding.Labels parameter".into(), "F2"));
    }
    let intrinsic = property(cx, non_nullable, "_flamework_intrinsic")?;
    if !tsrs_checker::is_tuple_type_exported(intrinsic) {
        return None;
    }
    let arguments = cx.checker().get_type_arguments(intrinsic);
    let (&id, inputs) = arguments.split_first()?;
    let id = string_literal(id)?;
    let input = inputs.first().copied();
    match (id, input) {
        ("path", Some(input)) => Some(UserMacro::Path(input)),
        ("pathglob", Some(input)) => Some(UserMacro::PathGlob(input)),
        (id, _) => {
            let milestone = match id {
                "serializer" => "F4",
                "network-decoder" | "network-result-decoder" | "network-unreliable" => "F5",
                "tuple-guards" => "F3",
                _ => "F2",
            };
            Some(UserMacro::Later(format!("the {id} intrinsic"), milestone))
        }
    }
}

/// The text of a string literal type.
fn string_literal(ty: P<Type>) -> Option<&'static str> {
    if !ty.flags().intersects(TypeFlags::StringLiteral) {
        return None;
    }
    match ty.as_literal_type().value() {
        Some(LiteralValue::String(s)) => Some(s),
        _ => None,
    }
}

/// `getMacroCall`: the macro's name and where it is, one-based, in UTF-16. A method call is placed at the method's
/// name (a chain of registrations is one expression, and every call in it starts where the chain does).
/// `qualified` also names the receiver when it is a plain name (`ComponentPlugin.fromPath`).
fn macro_call_shape(cx: &Cx<'_>, call: P<Node>, qualified: bool) -> (String, u32, u32) {
    let mut name = "a macro".to_owned();
    let mut anchor = call;
    if matches!(call.kind(), Kind::CallExpression | Kind::NewExpression)
        && let Some(callee) = call.expression()
    {
        match callee.kind() {
            Kind::PropertyAccessExpression => {
                let access = callee.as_property_access_expression();
                let method = access.name.text();
                let object = access.expression;
                name = if qualified && object.kind() == Kind::Identifier {
                    format!("{}.{method}", object.text())
                } else {
                    method.to_owned()
                };
                anchor = access.name;
            }
            Kind::Identifier => name = callee.text().to_owned(),
            _ => {}
        }
    }
    let pos = cx.utf16_pos(anchor);
    (name, pos.line + 1, pos.character + 1)
}

/// A path literal's text, or the TS transformer's error.
fn path_literal(cx: &mut Cx<'_>, node: P<Node>, ty: P<Type>) -> PResult<&'static str> {
    match string_literal(ty) {
        Some(value) => Ok(value),
        None => {
            let shown = cx.checker().type_to_string_exported(ty);
            Err(cx.error(node, format!("Path is invalid, expected string literal and got: {shown}")))
        }
    }
}

/// `buildPathIntrinsic`: the Rojo path of a source path, as an array of names, with the use recorded.
fn build_path(fw: &FwInstance, cx: &mut Cx<'_>, call: P<Node>, ty: P<Type>, key: OrderKey) -> PResult<LuauExpr> {
    let shared = &fw.shared;
    let value = path_literal(cx, call, ty)?;
    // Source paths are relative to the working directory the TS transformer runs in: the project's.
    let source = node_path::resolve(&[&shared.project_dir, value]);
    let output = shared.path_translator.get_output_path(&source);
    let rbx_path = if shared.has_rojo { cx.project().rojo_resolver.get_rbx_path_from_file_path(&output) } else { None };
    let Some(rbx_path) = rbx_path else {
        let shown = node_path::relative(&shared.project_dir, &output).replace('\\', "/");
        let hint = if shared.has_rojo {
            format!(
                "It compiles to '{shown}', and no $path in your Rojo project covers that. Give the source path of a folder the project maps, relative to the project, such as \"src/server/commands\"."
            )
        } else {
            "No Rojo project file was found, so a source path cannot be turned into a Rojo path.".to_owned()
        };
        let diagnostic =
            PluginDiagnostic::error(call, format!("Could not find Rojo data for '{value}'")).with_hint(hint);
        return Err(cx.report(diagnostic).expect("an error"));
    };

    let (macro_name, line, column) = macro_call_shape(cx, call, true);
    let raises = is_core_require_modules(cx, call);
    let mut file = fw.file.borrow_mut();
    let seq = file.next();
    file.events.push((key, seq, PathEvent::Path(PathUse { path: value.to_owned(), macro_name, line, column, raises })));
    drop(file);
    Ok(create::array(rbx_path.iter().map(|name| text_literal(name)).collect()))
}

/// `buildPathGlobIntrinsic`: the glob relative to the package root (obfuscated under obfuscation), recorded for
/// `globs.json`.
fn build_path_glob(
    fw: &FwInstance,
    cx: &mut Cx<'_>,
    call: P<Node>,
    ty: P<Type>,
    key: OrderKey,
) -> PResult<LuauExpr> {
    let shared = &fw.shared;
    let glob = path_literal(cx, call, ty)?;
    let file_name = cx.file().path.clone();
    let absolute = if glob.starts_with('.') {
        let resolved = node_path::resolve(&[&node_path::dirname(&file_name), glob]);
        node_path::relative(&shared.root_dir, &resolved).replace('\\', "/")
    } else {
        glob.to_owned()
    };
    let (macro_name, line, column) = macro_call_shape(cx, call, false);
    let mut file = fw.file.borrow_mut();
    let seq = file.next();
    let glob_use = GlobUse { glob: absolute.clone(), text: glob.to_owned(), macro_name, line, column };
    file.events.push((key, seq, PathEvent::Glob(glob_use)));
    drop(file);
    Ok(text_literal(&shared.obfuscate_text(&absolute, Some("addPaths"))))
}

/// `isCoreRequireModules`: the call is core's `requireModules`, which raises after five seconds rather than waiting.
fn is_core_require_modules(cx: &mut Cx<'_>, call: P<Node>) -> bool {
    if call.kind() != Kind::CallExpression {
        return false;
    }
    let signature = cx.checker().get_resolved_signature_exported(call);
    let Some(declaration) = signature.declaration() else { return false };
    if declaration.kind() != Kind::FunctionDeclaration
        || declaration.name().map(|n| n.text()) != Some("requireModules")
    {
        return false;
    }
    crate::class_hook::declaring_package(cx, declaration).as_deref() == Some(CORE_PACKAGE)
}
