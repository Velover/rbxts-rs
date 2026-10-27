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
/// the first overload, which rejects it on the return type; the second then refuses that `this`): TS2769 as stock
/// prints it, each overload's error listed (`rbxts_project`'s `overload_errors`), exit 1 and nothing written.
#[test]
fn an_overload_error_reports_ts2769_as_stock() {
    let Some(root) = diagnostics_like_stock("overload_error") else { return };
    assert!(!root.path().join("out").exists(), "nothing is written");
}

/// TS2769 for generic overloads: each listed overload is the one inference instantiated, as stock prints it.
#[test]
fn a_generic_overload_error_reports_ts2769_as_stock() {
    diagnostics_like_stock("overload_error_generic");
}

/// TS2769 with three overloads tried (a fourth ruled out by arity), and a method's two overloads: stock's text, the
/// implementation's elaboration and the line end stock prints after an empty related information list.
#[test]
fn a_three_overload_error_reports_ts2769_as_stock() {
    diagnostics_like_stock("overload_error_three");
}

/// TS2769 on `new`: a class's constructor overloads and an interface's `new` members, each written as a call
/// signature (no `new`), the error on the callee (`C`, a property access's name) where the overloads' errors differ.
#[test]
fn a_new_expression_overload_error_reports_ts2769_as_stock() {
    diagnostics_like_stock("overload_error_new");
}

/// TS2769 on a tagged template, the error on its tag.
#[test]
fn a_tagged_template_overload_error_reports_ts2769_as_stock() {
    diagnostics_like_stock("overload_error_tagged");
}

/// A class and a method decorator no overload takes: TS1238 / TS1241 over TS2769, each overload's error listed.
#[test]
fn a_decorator_overload_error_reports_as_stock() {
    diagnostics_like_stock("overload_error_decorator");
}

/// TS2769 on JSX elements: an overloaded function component and a component type with two call signatures.
#[test]
fn a_jsx_overload_error_reports_ts2769_as_stock() {
    diagnostics_like_stock("overload_error_jsx");
}

/// TS2683's related information on an anonymous function expression spans the function's first token (`function`,
/// `async`), as TypeScript 5 spans it, not the name it is assigned to (TypeScript 7).
#[test]
fn an_outer_this_is_shadowed_by_the_function_as_stock_spans_it() {
    diagnostics_like_stock("this_shadowed");
}

/// A known difference (docs/tsrs.md): a generator with an explicit `this` parameter whose `yield` is context sensitive
/// is context sensitive to TypeScript 7, not to TypeScript 5, so a generic call infers otherwise. `expected-stdout.txt`
/// is rbxts-rs's, `stock-stdout.txt` stock's: they differ in the one error's inferred type alone.
#[test]
fn a_context_sensitive_yield_differs_from_stock_as_documented() {
    diagnostics_like_stock("yield_context_sensitive");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/yield_context_sensitive");
    let ours = std::fs::read_to_string(fixture.join("expected-stdout.txt")).unwrap();
    let stock = std::fs::read_to_string(fixture.join("stock-stdout.txt")).unwrap();
    assert_eq!(ours.replacen("'string' is not assignable", "'unknown' is not assignable", 1), stock);
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
