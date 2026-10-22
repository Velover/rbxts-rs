//! `rbxts-probe property-order`: the property order of every class and interface declaration, annotated variable and
//! `createGuard<T>()` type argument in the matching files, as TypeScript 7 lists it (`--raw`) or as
//! `rbxts_transform::util::ts5_property_order` rebuilds TypeScript 5's. The lines have the format of the scratchpad's
//! `ts5props.cjs`, which prints TypeScript 5's own order, so the two diff directly.

use rbxts_project::LoadOptions;
use rbxts_transform::util::ts5_property_order::ts5_property_order;
use tsrs_ast::{Kind, Node, SourceFile};
use tsrs_checker::{Checker, Type};
use tsrs_compiler::Context;
use tsrs_core::P;

/// What the probe prints.
pub struct Options {
    pub tsconfig: String,
    pub filter: String,
    /// TypeScript 7's order instead of the rebuilt TypeScript 5 one.
    pub raw: bool,
    /// Check every file before reading any type (types resolved in check order rather than on demand).
    pub check_first: bool,
}

fn source_text(file: &SourceFile, node: P<Node>) -> &'static str {
    let (pos, end) = (node.pos().expect_source("parsed"), node.end().expect_source("parsed"));
    file.text()[pos as usize..end as usize].trim()
}

fn line_of(file: &SourceFile, node: P<Node>) -> usize {
    let start = tsrs_scanner::skip_trivia(file.text(), node.pos().expect_source("parsed")) as usize;
    file.text()[..start].bytes().filter(|&b| b == b'\n').count() + 1
}

fn props(checker: &mut Checker, t: P<Type>, raw: bool) -> String {
    let symbols =
        if raw { checker.get_properties_of_type_exported(t).to_vec() } else { ts5_property_order(checker, t) };
    symbols.iter().map(|s| s.name()).collect::<Vec<_>>().join(", ")
}

fn visit(checker: &mut Checker, file: &'static SourceFile, rel: &str, node: P<Node>, raw: bool) {
    match node.kind() {
        Kind::InterfaceDeclaration | Kind::ClassDeclaration => {
            if let Some(name) = node.name()
                && let Some(symbol) = checker.get_symbol_at_location_exported(name)
            {
                let t = checker.get_declared_type_of_symbol_exported(symbol);
                println!("{rel} decl {}: {}", name.text(), props(checker, t, raw));
            }
        }
        Kind::VariableDeclaration => {
            if let (Some(name), Some(type_node)) = (node.name(), node.type_node())
                && type_node.kind() == Kind::TypeReference
            {
                let t = checker.get_type_at_location(name);
                println!("{rel} var {}: {}", source_text(file, name), props(checker, t, raw));
            }
        }
        Kind::CallExpression => {
            let callee = node.expression().expect("a call has a callee");
            if source_text(file, callee).ends_with("createGuard")
                && let Some(&argument) = node.type_arguments().first()
            {
                let t = checker.get_type_from_type_node_exported(argument);
                let line = line_of(file, node);
                println!("{rel}:{line} guard {}: {}", source_text(file, argument), props(checker, t, raw));
            }
        }
        _ => {}
    }
    node.for_each_child(&mut |child| {
        visit(checker, file, rel, child, raw);
        false
    });
}

/// Runs the probe; the exit status.
pub fn run(opts: &Options) -> i32 {
    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let project =
        match rbxts_project::load(&LoadOptions { tsconfig: opts.tsconfig.clone(), cwd, checkers: 1, lazy_dts: false }) {
            Ok(p) => p,
            Err(diags) => {
                eprintln!("tsconfig could not be used ({} diagnostics)", diags.len());
                return 2;
            }
        };
    let ctx = Context::background();
    project.program.bind_source_files();
    let mut checker = project.program.get_type_checker(&ctx);
    if opts.check_first {
        for &file in project.program.source_files() {
            project.program.get_semantic_diagnostics_with_checker(&ctx, &mut checker, file);
        }
    }
    let base = std::path::Path::new(&opts.tsconfig)
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_string_lossy().replace('\\', "/")))
        .unwrap_or_default();
    let base = base.trim_start_matches("//?/").to_owned();
    println!("# tsrs ({})", if opts.raw { "TypeScript 7 order" } else { "ts5_property_order" });
    for &file in project.program.source_files() {
        let name = file.file_name();
        if !name.contains(&opts.filter) {
            continue;
        }
        let rel = name.strip_prefix(&base).map(|r| r.trim_start_matches('/')).unwrap_or(name).to_owned();
        visit(&mut checker, file.get(), &rel, file.as_node(), opts.raw);
    }
    0
}
