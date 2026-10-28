//! A plugin error beside checker errors, against stock roblox-ts's stdout (`tests/fixtures/forwarding`): each case of
//! `cases/<case>` built with the probe in its fail mode, plain (`probe-fail`) and forwarding checker diagnostics as
//! Flamework's TS transformer does (`probe-fail-forward`), the stdout compared byte for byte with roblox-ts master's
//! on TypeScript 5.9 (`node out/CLI/cli.js build`) running `stock-transformer.cjs`, a JavaScript transformer that
//! mirrors the probe's ops (its tsconfig entry with `"forward": false` / `true`): `expected/<case>-leave.txt` and
//! `expected/<case>-forward.txt`.
//!
//! roblox-ts's `compileFiles` runs its transformers over every file before checking any, and returns with their
//! diagnostics alone when one is an error: a plain transformer's error is reported alone (c1, c2, c5, c6), and without
//! one the check and roblox-ts's own transform are reported, cut at the first error, with the transformer's warnings
//! of every file (c4, c8, c10); a forwarding transformer's report is the failing files' checker diagnostics (explained)
//! with what it reports on the files that pass, and nothing of roblox-ts's own (every case).
//!
//! Not as stock (and not tested): a plain transformer's diagnostics on a file that fails checking (c3, c7, c9), as no
//! plugin transforms such a file; rbxts-rs reports what the plugins report on the other files instead.

#![cfg(windows)]

use std::path::Path;

use rbxts_project::{ProjectOptionLayer, STACK_SIZE};
use rbxts_test_support::{TempDir, copy_dir, node_modules_or_skip};
use rbxts_transform::{CompileOptions, PluginOptions, compile_project_with};
use tsrs_compiler::sort_and_deduplicate_diagnostics;

/// The stdout `rbxtsc build` prints for case `case` built with the test plugin `plugin`, as `Cli` prints it.
fn stdout_of(case: &str, plugin: &str) -> Option<String> {
    let node_modules = node_modules_or_skip(&format!("forwarding::{case}"))?;
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/forwarding");
    let mut root = TempDir::new(&format!("forwarding-{case}-{plugin}"));
    for name in ["src", "tsconfig.json", "default.project.json", "package.json"] {
        let from = fixture.join(name);
        if from.is_dir() {
            copy_dir(&from, &root.path().join(name));
        } else {
            std::fs::copy(&from, root.path().join(name)).unwrap();
        }
    }
    copy_dir(&fixture.join("cases").join(case).join("src"), &root.path().join("src"));
    root.link("node_modules", &node_modules);
    let cwd = root.path().to_string_lossy().into_owned();
    let plugins = vec![rbxts_test_plugins::by_name(plugin).unwrap()];
    let text = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, || {
                let options = CompileOptions {
                    tsconfig: "tsconfig.json".into(),
                    cwd: cwd.clone(),
                    project_options: ProjectOptionLayer::default(),
                    out_dir_override: None,
                    write_partial: false,
                    no_write: false,
                    checkers: None,
                };
                let report =
                    compile_project_with(&options, &PluginOptions { plugins: &plugins, ..PluginOptions::default() });
                assert!(report.fatal.is_empty(), "{case} {plugin}: {:?}", report.fatal);
                // as `print_report`: sorted and deduplicated, each with TypeScript's pretty reporter and a line end
                let mut diagnostics = report.diagnostics.clone();
                for file in &report.files {
                    diagnostics.extend(file.diagnostics.iter().copied());
                }
                sort_and_deduplicate_diagnostics(&diagnostics)
                    .into_iter()
                    .map(|d| format!("{}\r\n", rbxts_cli::format_diagnostic(d, &cwd)))
                    .collect::<String>()
            })
            .unwrap()
            .join()
            .unwrap()
    });
    assert!(!root.path().join("out").exists(), "{case} {plugin}: nothing is written");
    Some(text)
}

fn expected(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/forwarding/expected").join(name);
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_plain_plugin_error_is_reported_alone_as_stock() {
    for case in ["c1", "c2", "c4", "c5", "c6", "c8", "c10"] {
        let Some(ours) = stdout_of(case, "probe-fail") else {
            return;
        };
        assert_eq!(ours, expected(&format!("{case}-leave.txt")), "{case}");
    }
}

#[test]
fn forwarded_checker_diagnostics_are_reported_with_the_plugins_as_stock() {
    for case in ["c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8", "c9", "c10"] {
        let Some(ours) = stdout_of(case, "probe-fail-forward") else {
            return;
        };
        assert_eq!(ours, expected(&format!("{case}-forward.txt")), "{case}");
    }
}
