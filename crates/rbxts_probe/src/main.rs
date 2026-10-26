//! Probes of tsrs behaviour the design depends on (milestone M0b). Each prints what it found; none changes anything.
//!
//! rbxts-probe jsdoc-links [-p tsconfig.json] [--files <path substring>]
//!   Resolves every JSDoc `{@link name}` in the matching files (default: `@flamework-experimental`) with the checker,
//!   as Flamework's `@metadata macro {@link param intrinsic-...}` detection does.
//!
//! rbxts-probe property-order [-p tsconfig.json] [--files <path substring>] [--raw] [--check-first]
//!   The property order of classes, interfaces, annotated variables and `createGuard<T>()` type arguments: TypeScript
//!   7's (`--raw`) or the rebuilt TypeScript 5 one (`ts5_property_order`). See `property_order.rs`.

mod property_order;

#[global_allocator]
static GLOBAL: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;

use rbxts_project::{LoadOptions, STACK_SIZE};
use tsrs_ast::{Kind, Node, SourceFile};
use tsrs_compiler::Context;
use tsrs_core::P;

fn collect_links(node: P<Node>, out: &mut Vec<P<Node>>) {
    if node.kind() == Kind::JSDocLink {
        out.push(node);
    }
    node.for_each_child(&mut |child| {
        collect_links(child, out);
        false
    });
}

/// Every `{@link}` in the JSDoc of `node` and its descendants, with the node the JSDoc belongs to.
fn walk(file: &'static SourceFile, node: P<Node>, out: &mut Vec<(P<Node>, P<Node>)>) {
    for &jsdoc in node.jsdoc(Some(file)) {
        let mut links = Vec::new();
        collect_links(jsdoc, &mut links);
        out.extend(links.into_iter().map(|l| (node, l)));
    }
    node.for_each_child(&mut |child| {
        walk(file, child, out);
        false
    });
}

/// The node's source text (`Node::text` does not cover every name kind, e.g. `QualifiedName`).
fn source_text(file: &SourceFile, node: P<Node>) -> &'static str {
    let (pos, end) = (node.pos().expect_source("parsed"), node.end().expect_source("parsed"));
    file.text()[pos as usize..end as usize].trim()
}

fn line_of(file: &SourceFile, pos: u32) -> usize {
    file.text()[..pos as usize].bytes().filter(|&b| b == b'\n').count() + 1
}

fn jsdoc_links(tsconfig: String, filter: &str) -> i32 {
    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let project = match rbxts_project::load(&LoadOptions { tsconfig, cwd, checkers: 1, lazy_dts: false, regions: false }) {
        Ok(p) => p,
        Err(diags) => {
            eprintln!("tsconfig could not be used ({} diagnostics)", diags.len());
            return 2;
        }
    };
    let ctx = Context::background();
    project.program.bind_source_files();
    let mut checker = project.program.get_type_checker(&ctx);
    let (mut resolved, mut unresolved, mut files) = (0, 0, 0);
    for &file in project.program.source_files() {
        if !file.file_name().contains(filter) {
            continue;
        }
        let mut links = Vec::new();
        walk(file.get(), file.as_node(), &mut links);
        if links.is_empty() {
            continue;
        }
        files += 1;
        println!("{} ({} links)", file.file_name(), links.len());
        for (host, link) in links {
            let Some(name) = link.as_jsdoc_link().name() else { continue };
            let line = line_of(&file, link.pos().expect_source("parsed"));
            match checker.get_symbol_at_location_exported(name) {
                Some(symbol) => {
                    resolved += 1;
                    let decl = symbol.declarations().first().map(|d| format!("{:?}", d.kind())).unwrap_or_default();
                    // Structural: the linked symbol's declaration is a parameter of the JSDoc host. Flamework matches by
                    // symbol identity against the signature's parameters; the two agree when the JSDoc sits on the
                    // signature itself, as in every Flamework package.
                    let is_param_of_host = symbol.value_declaration().is_some_and(|d| {
                        d.kind() == Kind::Parameter && d.parent().is_some_and(|p| p == host)
                    });
                    println!(
                        "  :{line} {{@link {}}} -> {} ({decl}){} [host {:?}]",
                        source_text(&file, name),
                        symbol.name(),
                        if is_param_of_host { ", a parameter of the host" } else { "" },
                        host.kind()
                    );
                }
                None => {
                    unresolved += 1;
                    println!("  :{line} {{@link {}}} -> UNRESOLVED [host {:?}]", source_text(&file, name), host.kind());
                }
            }
        }
    }
    println!("files {files}, links resolved {resolved}, unresolved {unresolved}");
    i32::from(unresolved > 0)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let (mut tsconfig, mut filter) = ("tsconfig.json".to_owned(), "@flamework-experimental".to_owned());
    let (mut raw, mut check_first) = (false, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--raw" => {
                raw = true;
                continue;
            }
            "--check-first" => {
                check_first = true;
                continue;
            }
            _ => {}
        }
        match (arg.as_str(), args.next()) {
            ("-p" | "--project", Some(v)) => tsconfig = v,
            ("--files", Some(v)) => filter = v,
            _ => {
                eprintln!("rbxts-probe: bad argument {arg}");
                std::process::exit(2);
            }
        }
    }
    let status = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || match command.as_str() {
            "jsdoc-links" => jsdoc_links(tsconfig, &filter),
            "property-order" => property_order::run(&property_order::Options { tsconfig, filter, raw, check_first }),
            other => {
                eprintln!("rbxts-probe: unknown command {other:?}; expected jsdoc-links or property-order");
                2
            }
        })
        .expect("spawn the probe thread")
        .join()
        .unwrap_or(101);
    std::process::exit(status);
}
