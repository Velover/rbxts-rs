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
fn diagnostics_like_stock(name: &str) -> Option<TempDir> {
    let node_modules = node_modules_or_skip(&format!("cli::{name}"))?;
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let mut root = TempDir::new(&format!("cli-{name}"));
    for name in ["src", "tsconfig.json", "package.json", "typings"] {
        let from = fixture.join(name);
        if !from.exists() {
            continue;
        }
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
    Some(root)
}

#[test]
fn diagnostics_print_as_stock_with_related_information() {
    diagnostics_like_stock("diagnostics_pretty");
}

#[test]
fn diagnostics_print_as_stock_with_an_empty_span() {
    diagnostics_like_stock("diagnostics_pretty_syntax");
}

/// An overloaded call no overload takes, a function expression argument deciding it (TypeScript 5 fixes its `this` from
/// the first overload, which rejects it on the return type; the second then refuses that `this`): TS2769 at stock's
/// place, exit 1 and nothing written. Its elaboration is TypeScript 7's, which reports only the last overload's
/// error where TypeScript 5 lists each of two or three (`stock-stdout.txt` is stock's; docs/tsrs.md).
#[test]
fn an_overload_error_reports_ts2769_as_stock() {
    let Some(root) = diagnostics_like_stock("overload_error") else { return };
    assert!(!root.path().join("out").exists(), "nothing is written");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overload_error");
    let ours = std::fs::read_to_string(fixture.join("expected-stdout.txt")).unwrap();
    let stock = std::fs::read_to_string(fixture.join("stock-stdout.txt")).unwrap();
    let (ours, stock): (Vec<&str>, Vec<&str>) = (ours.lines().collect(), stock.lines().collect());
    assert_eq!(ours[0], stock[0], "the error line (place, TS2769, message) is stock's");
    // from the code frame on, stock's but for TypeScript 7's "The last overload is declared here" (4 lines)
    let frame = |lines: &[&str]| lines.iter().position(|l| l.starts_with("[7m11")).unwrap();
    let mut ours_tail = ours[frame(&ours)..].to_vec();
    let last = ours_tail.iter().position(|l| l.contains("The last overload is declared here")).unwrap();
    ours_tail.drain(last - 3..=last);
    assert_eq!(ours_tail, stock[frame(&stock)..].to_vec());
}

/// roblox-ts's `compileFiles` adds every file's checker diagnostics but transforms file by file, in transform order,
/// and transforms nothing after the first error: `b.ts`'s roblox-ts error and `d.ts`'s checker error are reported,
/// `c.ts`'s roblox-ts error is not (it comes after `b.ts`'s).
#[test]
fn diagnostics_stop_at_the_first_file_with_an_error_as_stock() {
    diagnostics_like_stock("diagnostics_first_error");
}

/// A type-root package without declarations, the tsconfig without `types`: TypeScript 5's TS2688 wording ("Entry point
/// for implicit type library").
#[test]
fn a_type_root_package_without_declarations_reports_as_stock() {
    diagnostics_like_stock("automatic_types_no_declarations");
}

/// `"types": []`: roblox-ts makes its MacroManager before checking anything, so the compiler types it cannot find
/// are its only error (no TS2318).
#[test]
fn empty_types_stop_at_the_macro_manager_as_stock() {
    diagnostics_like_stock("types_empty");
}

/// `baseUrl` with `paths`: a name a `paths` pattern matches is not tried against `baseUrl` when no substitution resolves
/// (TypeScript 5's rule, which `rbxts_project`'s `baseUrl` emulation keeps): TS2307 as stock.
#[test]
fn base_url_does_not_back_a_matched_paths_pattern_as_stock() {
    diagnostics_like_stock("base_url_paths_no_fallback");
}

/// `"types": []` and no source to transform (a declaration file alone): stock still makes its MacroManager, so its
/// error is the build's.
#[test]
fn empty_types_without_sources_stop_at_the_macro_manager_as_stock() {
    diagnostics_like_stock("types_empty_no_sources");
}

/// Without `types` in the tsconfig, every package of the type roots is in the program, as in TypeScript 5: a project
/// whose sources import no package still sees `@rbxts/compiler-types` (TypeScript 7 would load none: TS2318).
#[test]
fn a_tsconfig_without_types_loads_every_type_root_package_as_stock() {
    let Some(node_modules) = node_modules_or_skip("cli::automatic_types") else { return };
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/automatic_types");
    let mut root = TempDir::new("cli-automatic-types");
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
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(output.status.code(), Some(0));
    assert!(root.path().join("out/init.luau").is_file());
}
