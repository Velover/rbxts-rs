//! Builds a project with rbxts-rs and the Flamework plugin, for parity runs against TS5 output.
//!
//! `cargo run -p flamework_rs --example fw_build -- <project dir> [--probe] [--partial]`
//!
//! - `--probe`: Flamework with `Flamework.createGuard<T>()` built by the guard generator (`guards::probe`), F2's
//!   `Generic<T, "guard">` stand-in.
//! - `--partial`: write the files that built even when others failed.
//!
//! Prints every diagnostic as `error|warning <file>:<line>:<column>: <message>` (UTF-16 columns from 1, as
//! TypeScript prints them), each related note indented below it, then the warnings and fatal errors, and `files=N`.

use std::path::Path;

use flamework_rs::Flamework;
use flamework_rs::guards::probe::GuardProbe;
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let project = args.first().expect("usage: fw_build <project dir> [--probe] [--partial]").clone();
    let probe = args.iter().any(|a| a == "--probe");
    let partial = args.iter().any(|a| a == "--partial");
    let cwd = Path::new(&project).canonicalize().expect("project dir").to_string_lossy().replace("\\\\?\\", "");

    let report = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let plugins: Vec<Box<dyn DynPlugin>> =
                if probe { vec![erase(GuardProbe::new())] } else { vec![erase(Flamework::new())] };
            compile_project_with(
                &CompileOptions {
                    tsconfig: "tsconfig.json".into(),
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
        .unwrap()
        .join()
        .unwrap();

    let mut all = report.diagnostics.clone();
    for file in &report.files {
        all.extend(file.diagnostics.iter().copied());
    }
    for d in all {
        let category = if d.category() == Category::Error { "error" } else { "warning" };
        println!("{category} {}: {}", position(d), flatten_diagnostic_message(d, "\n"));
        for &related in d.related_information() {
            println!("    {}: {}", position(related), flatten_diagnostic_message(related, "\n"));
        }
    }
    for warning in &report.warnings {
        println!("warning: {warning}");
    }
    for fatal in &report.fatal {
        println!("fatal: {fatal}");
    }
    let written = report.files.iter().filter(|f| f.luau.is_some()).count();
    println!("files={} luau={written}", report.files.len());
    std::process::exit(if report.fatal.is_empty() && !report.files.iter().any(|f| f.diagnostics.iter().any(|d| d.category() == Category::Error)) { 0 } else { 1 });
}
