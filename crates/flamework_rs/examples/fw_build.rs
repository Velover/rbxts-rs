//! Builds a project with rbxts-rs and the Flamework plugin, for parity runs against TS5 output. The transformer
//! fixture's macro types are registered (`tests/support/fixture_macros.rs`), so its `transformer.plugins` macros
//! build too; a project that uses none of them builds as with the plain plugin.
//!
//! `cargo run -p flamework_rs --example fw_build -- <project dir> [tsconfig.json] [--partial]`
//!
//! - `--partial`: write the files that built even when others failed.
//!
//! Writes the project's `out`, `include/flamework` and `flamework.build` as a build does. Prints every diagnostic as
//! `error|warning <file>:<line>:<column>: <message>` (UTF-16 columns from 1, as TypeScript prints them), each related
//! note indented below it, then the warnings and fatal errors, and `files=N luau=N errors=N ms=N`. Exits 1 when the
//! build has errors.

#[path = "../tests/support/fixture_macros.rs"]
mod fixture_macros;

use std::path::Path;

use rbxts_plugin::erased::{DynPlugin, erase};
use rbxts_plugin::ts::{Diagnostic, P};
use rbxts_project::{ProjectOptionLayer, STACK_SIZE};
use rbxts_transform::{CompileOptions, PluginOptions, compile_project_with};
use tsrs_compiler::diagnosticwriter::flatten_diagnostic_message;
use tsrs_diagnostics::Category;

fn position(d: P<Diagnostic>) -> String {
    let Some(file) = d.file() else { return String::new() };
    let text = file.get().text();
    let pos = d.pos().unwrap_or(0) as usize;
    // From the diagnostic's start, skip trivia as `getStart` does (the plugin reports from `getStart`).
    let before = &text[..pos.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = text[line_start..pos.min(text.len())].encode_utf16().count() + 1;
    let name = file.get().file_name().to_owned();
    format!("{name}:{line}:{column}")
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let partial = args.iter().any(|a| a == "--partial");
    let mut positional = args.iter().filter(|a| !a.starts_with("--"));
    let Some(project) = positional.next() else {
        eprintln!("usage: fw_build <project dir> [tsconfig.json] [--partial]");
        return std::process::ExitCode::from(2);
    };
    let tsconfig = positional.next().cloned().unwrap_or_else(|| "tsconfig.json".into());
    let cwd = Path::new(project).canonicalize().expect("project dir").to_string_lossy().replace("\\\\?\\", "");

    let start = std::time::Instant::now();
    let report = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let plugins: Vec<Box<dyn DynPlugin>> = vec![erase(fixture_macros::flamework_with_fixture_macros())];
            compile_project_with(
                &CompileOptions {
                    tsconfig,
                    cwd,
                    project_options: ProjectOptionLayer {
                        optimized_loops: Some(true),
                        allow_comment_directives: Some(true),
                        ..ProjectOptionLayer::default()
                    },
                    out_dir_override: None,
                    write_partial: partial,
                    no_write: false,
                },
                &PluginOptions { plugins: &plugins, ..PluginOptions::default() },
            )
        })
        .expect("a build thread")
        .join()
        .expect("the build finished");

    let mut errors = 0;
    let mut all = report.diagnostics.clone();
    for file in &report.files {
        all.extend(file.diagnostics.iter().copied());
    }
    for d in all {
        let category = if d.category() == Category::Error {
            errors += 1;
            "error"
        } else {
            "warning"
        };
        println!("{category} {}: {}", position(d), flatten_diagnostic_message(d, "\n"));
        for &related in d.related_information() {
            println!("    {}: {}", position(related), flatten_diagnostic_message(related, "\n"));
        }
    }
    for warning in &report.warnings {
        println!("warning: {warning}");
    }
    for fatal in &report.fatal {
        errors += 1;
        println!("fatal: {fatal}");
    }
    let luau = report.files.iter().filter(|f| f.luau.is_some()).count();
    println!("files={} luau={luau} errors={errors} ms={}", report.files.len(), start.elapsed().as_millis());
    if errors > 0 { std::process::ExitCode::FAILURE } else { std::process::ExitCode::SUCCESS }
}
