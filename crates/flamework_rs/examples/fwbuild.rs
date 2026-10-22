//! Builds a project with rbxts-rs and Flamework, with the fixture's macro types registered (`tests/support`): the
//! F2 parity builds compare its output with the TS5 pipeline's.
//!
//! `cargo run -p flamework_rs --example fwbuild -- <project dir> [tsconfig.json]`
//!
//! Writes the project's `out`, `include/flamework` and `flamework.build` as a build does, and prints every
//! diagnostic and warning. Exits 1 when the build has errors.

#[path = "../tests/support/fixture_macros.rs"]
mod fixture_macros;

use rbxts_plugin::erased::{DynPlugin, erase};
use rbxts_project::{ProjectOptionLayer, STACK_SIZE};
use rbxts_transform::{CompileOptions, PluginOptions, compile_project_with};
use tsrs_compiler::diagnosticwriter::flatten_diagnostic_message;
use tsrs_diagnostics::Category;

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(cwd) = args.next() else {
        eprintln!("usage: fwbuild <project dir> [tsconfig.json]");
        return std::process::ExitCode::from(2);
    };
    let tsconfig = args.next().unwrap_or_else(|| "tsconfig.json".into());
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
                    write_partial: false,
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
    for d in &all {
        let category = if d.category() == Category::Error {
            errors += 1;
            "error"
        } else {
            "warning"
        };
        let file = d.file().map(|f| f.file_name().to_owned()).unwrap_or_default();
        println!("{category} {file}: {}", flatten_diagnostic_message(*d, "\n"));
    }
    for warning in &report.warnings {
        println!("warning: {warning}");
    }
    for fatal in &report.fatal {
        errors += 1;
        println!("fatal: {fatal}");
    }
    let luau = report.files.iter().filter(|f| f.luau.is_some()).count();
    println!("files {} luau {luau} errors {errors} ms {}", report.files.len(), start.elapsed().as_millis());
    if errors > 0 { std::process::ExitCode::FAILURE } else { std::process::ExitCode::SUCCESS }
}
