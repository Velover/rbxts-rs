//! `Cli` end to end on a copy of `rbxts_transform`'s probe fixture: registered plugins build it (exit 0), and the
//! plain command line, without the plugin the tsconfig names, refuses it (exit 1), as roblox-ts's exit codes go.
//! Then the `rbxtsc` binary's diagnostics against stock roblox-ts's stdout, byte for byte
//! (`tests/fixtures/diagnostics_*/expected-stdout.txt`: roblox-ts master on TypeScript 5, `node out/CLI/cli.js build`):
//! related information, chained messages, an empty span, spans of several lines and of more than five, tabs and
//! characters outside the BMP before a span.

#![cfg(windows)]

use std::path::Path;

use rbxts_test_support::{TempDir, copy_dir, node_modules_or_skip};

#[test]
fn builds_with_registered_plugins_and_returns_roblox_ts_exit_codes() {
    let Some(node_modules) = node_modules_or_skip("cli::builds_with_registered_plugins") else { return };
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../rbxts_transform/tests/plugin_fixtures/probe");
    let mut root = TempDir::new("cli");
    for name in ["src", "tsconfig.json", "default.project.json", "package.json"] {
        let from = fixture.join(name);
        if from.is_dir() {
            copy_dir(&from, &root.path().join(name));
        } else {
            std::fs::copy(&from, root.path().join(name)).unwrap();
        }
    }
    root.link("node_modules", &node_modules);
    let project = root.path().to_string_lossy().into_owned();

    // the tsconfig lists rbxts-transform-stamp, which the plain command line does not have
    let plain = rbxts_cli::Cli::new().run_with(vec!["build".into(), "-p".into(), project.clone()]);
    assert_eq!(plain, 1);
    assert!(!root.path().join("out").exists());

    let status = rbxts_cli::Cli::new()
        .erased_plugin(rbxts_test_plugins::by_name("stamp").unwrap())
        .erased_plugin(rbxts_test_plugins::by_name("probe").unwrap())
        .run_with(vec!["-p".into(), project.clone(), "--timings".into()]);
    assert_eq!(status, 0);
    assert!(root.path().join("out/shared/calls.luau").is_file());
    assert!(root.path().join("stamp-report.json").is_file());

    // a hook error fails the build: exit 1
    let status = rbxts_cli::Cli::new()
        .erased_plugin(rbxts_test_plugins::by_name("stamp").unwrap())
        .erased_plugin(rbxts_test_plugins::by_name("probe-fail").unwrap())
        .run_with(vec!["-p".into(), project]);
    assert_eq!(status, 1);
}

/// `rbxtsc build` in a copy of `tests/fixtures/<name>`: stdout as stock prints it, exit code 1.
fn diagnostics_like_stock(name: &str) {
    let Some(node_modules) = node_modules_or_skip(&format!("cli::{name}")) else { return };
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let mut root = TempDir::new(&format!("cli-{name}"));
    for name in ["src", "tsconfig.json", "package.json"] {
        let from = fixture.join(name);
        if from.is_dir() {
            copy_dir(&from, &root.path().join(name));
        } else {
            std::fs::copy(&from, root.path().join(name)).unwrap();
        }
    }
    root.link("node_modules", &node_modules);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rbxtsc"))
        .arg("build")
        .current_dir(root.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let expected = std::fs::read_to_string(fixture.join("expected-stdout.txt")).unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
}

#[test]
fn diagnostics_print_as_stock_with_related_information() {
    diagnostics_like_stock("diagnostics_pretty");
}

#[test]
fn diagnostics_print_as_stock_with_an_empty_span() {
    diagnostics_like_stock("diagnostics_pretty_syntax");
}
