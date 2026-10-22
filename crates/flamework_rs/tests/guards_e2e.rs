//! Guards (F3) end to end, through the real compiler on `tests/guard_fixture` (one file per type kind) and
//! `tests/guard_errors` (the types the builder refuses).
//!
//! `tests/guard_fixture/expected` is rbxts-rs's output, checked against a TS5 build of the same sources (roblox-ts
//! master with the monorepo transformer at 1a72fa1, 2.0.0-alpha.8): every guard is TS5's text but for the order of
//! union members, which follows TS7 (DESIGN §10.4), and the differences listed in `crate::guards`' docs.
//!
//! `createGuard<T>()` calls go through `guards::probe::GuardProbe` (Flamework with F2's `Generic<T, "guard">` stood
//! in for); class metadata guards (`metadata.ts`) through the class hook, also checked with the plain `Flamework`
//! plugin. The project is built once and shared by the tests, in a temporary copy with a `node_modules` made as
//! `e2e.rs` makes it. Without a monorepo the tests are skipped, saying so.

#![cfg(windows)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use flamework_rs::Flamework;
use flamework_rs::guards::probe::GuardProbe;
use rbxts_plugin::erased::{DynPlugin, erase};
use rbxts_plugin::ts::{Diagnostic, P};
use rbxts_project::{ProjectOptionLayer, STACK_SIZE};
use rbxts_transform::{CompileOptions, CompileReport, PluginOptions, compile_project_with};
use tsrs_compiler::diagnosticwriter::flatten_diagnostic_message;
use tsrs_diagnostics::Category;

const DEFAULT_MONOREPO: &str = "E:/Projects/TS/Flamework/ExperimentalV2Monorepo";
const DEFAULT_RBXTS: &str = "E:/Projects/TS/Flamework/roblox-ts-test/realproject/node_modules/@rbxts";
const RBXTS_PACKAGES: [&str; 3] = ["compiler-types", "t", "types"];

fn sources() -> Option<(PathBuf, PathBuf)> {
    let var = |name: &str, default: &str| {
        std::env::var_os(name).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(default))
    };
    let monorepo = var("FLAMEWORK_RS_MONOREPO", DEFAULT_MONOREPO);
    let rbxts = var("FLAMEWORK_RS_RBXTS", DEFAULT_RBXTS);
    (monorepo.join("packages/core/out/index.d.ts").is_file()
        && RBXTS_PACKAGES.iter().all(|name| rbxts.join(name).is_dir()))
    .then_some((monorepo, rbxts))
}

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// What a build left: each output's Luau by its path under `out`, and the diagnostics as
/// `error|warning <file>:<line>:<column>: <message>` with related notes indented below.
#[derive(Debug, Default)]
struct Built {
    luau: BTreeMap<String, String>,
    diagnostics: Vec<String>,
    fatal: Vec<String>,
}

fn position(d: P<Diagnostic>) -> String {
    let Some(file) = d.file() else { return String::new() };
    let text = file.get().text();
    let pos = (d.pos().unwrap_or(0) as usize).min(text.len());
    let line = text[..pos].matches('\n').count() + 1;
    let line_start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    let column = text[line_start..pos].encode_utf16().count() + 1;
    let name = file.get().file_name().rsplit('/').next().unwrap_or_default().to_owned();
    format!("{name}:{line}:{column}")
}

fn build(fixture: &str, probe: bool) -> Option<Built> {
    let Some((monorepo, rbxts)) = sources() else {
        eprintln!("skipped: no monorepo with a built core, or no @rbxts packages (set FLAMEWORK_RS_MONOREPO, FLAMEWORK_RS_RBXTS)");
        return None;
    };
    let root = std::env::temp_dir().join(format!("flamework-rs-guards-{fixture}-{probe}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let from = tests_dir().join(fixture);
    for name in ["src", "tsconfig.json", "default.project.json", "package.json", "flamework.config.json"] {
        if from.join(name).is_dir() {
            copy_dir(&from.join(name), &root.join(name));
        } else {
            std::fs::copy(from.join(name), root.join(name)).unwrap();
        }
    }
    let node_modules = root.join("node_modules");
    for name in RBXTS_PACKAGES {
        copy_dir(&rbxts.join(name), &node_modules.join("@rbxts").join(name));
    }
    for name in ["core", "components", "networking"] {
        let from = monorepo.join("packages").join(name);
        let to = node_modules.join("@flamework-experimental").join(name);
        copy_dir(&from.join("out"), &to.join("out"));
        std::fs::copy(from.join("package.json"), to.join("package.json")).unwrap();
        // The packages as this transformer's version built them (the monorepo may be at the next release).
        let text = std::fs::read_to_string(from.join("flamework.build")).unwrap();
        let marker = "\"flameworkVersion\": \"";
        let start = text.find(marker).expect("a flameworkVersion") + marker.len();
        let end = start + text[start..].find('"').unwrap();
        let text = format!("{}{}{}", &text[..start], flamework_rs::VERSION, &text[end..]);
        std::fs::write(to.join("flamework.build"), text).unwrap();
    }

    let cwd = root.to_string_lossy().into_owned();
    let built = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let plugins: Vec<Box<dyn DynPlugin>> =
                if probe { vec![erase(GuardProbe::new())] } else { vec![erase(Flamework::new())] };
            let report: CompileReport = compile_project_with(
                &CompileOptions {
                    tsconfig: "tsconfig.json".into(),
                    cwd,
                    project_options: ProjectOptionLayer {
                        optimized_loops: Some(true),
                        allow_comment_directives: Some(true),
                        ..ProjectOptionLayer::default()
                    },
                    out_dir_override: None,
                    write_partial: false,
                    no_write: true,
                },
                &PluginOptions { plugins: &plugins, ..PluginOptions::default() },
            );
            let mut built = Built { fatal: report.fatal.clone(), ..Built::default() };
            let mut all = report.diagnostics.clone();
            for file in &report.files {
                all.extend(file.diagnostics.iter().copied());
                let relative = Path::new(&file.out_path)
                    .strip_prefix(&report.out_dir)
                    .unwrap_or(Path::new(&file.out_path))
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Some(luau) = &file.luau {
                    built.luau.insert(relative, luau.clone());
                }
            }
            for d in all {
                let category = if d.category() == Category::Error { "error" } else { "warning" };
                built.diagnostics.push(format!("{category} {}: {}", position(d), flatten_diagnostic_message(d, "\n")));
                for &related in d.related_information() {
                    built.diagnostics.push(format!("    {}: {}", position(related), flatten_diagnostic_message(related, "\n")));
                }
            }
            built
        })
        .unwrap()
        .join()
        .unwrap();
    let _ = std::fs::remove_dir_all(&root);
    Some(built)
}

/// The fixture built once with the probe, shared by the per-kind tests.
fn fixture() -> Option<&'static Built> {
    static BUILT: OnceLock<Option<Built>> = OnceLock::new();
    BUILT.get_or_init(|| build("guard_fixture", true)).as_ref()
}

/// One kind's output equals `expected/out/shared/<file>.luau`; `UPDATE_EXPECTED=1` writes it instead.
fn check(file: &str) {
    let Some(built) = fixture() else { return };
    assert!(built.fatal.is_empty(), "{:?}", built.fatal);
    let errors: Vec<&String> = built.diagnostics.iter().filter(|d| d.starts_with("error")).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let relative = format!("shared/{file}.luau");
    let ours = built.luau.get(&relative).unwrap_or_else(|| panic!("no output {relative}"));
    let expected_path = tests_dir().join("guard_fixture/expected/out").join(&relative);
    if std::env::var_os("UPDATE_EXPECTED").is_some() {
        std::fs::create_dir_all(expected_path.parent().unwrap()).unwrap();
        std::fs::write(&expected_path, ours).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|e| panic!("{}: {e}", expected_path.display()))
        .replace("\r\n", "\n");
    assert!(*ours == expected, "{relative} differs from expected\n--- ours\n{ours}");
}

#[test]
fn primitives_and_literals() {
    check("primitives");
}

#[test]
fn arrays_tuples_maps_and_records() {
    check("collections");
}

#[test]
fn objects_and_keys() {
    check("objects");
}

#[test]
fn typescript_and_roblox_enums() {
    check("enums");
}

#[test]
fn instance_types_and_children() {
    check("instances");
}

#[test]
fn template_literals() {
    check("templates");
}

#[test]
fn recursive_types() {
    check("recursive");
}

#[test]
fn generics_through_the_checker() {
    check("generics");
}

#[test]
fn spec_type_check_types() {
    check("specTypes");
}

#[test]
fn dedup_heuristic() {
    check("dedup");
}

#[test]
fn class_metadata_guards() {
    check("metadata");
}

/// The class hook's guards come from the plain plugin too (no probe): `metadata.luau` is the same.
#[test]
fn class_metadata_guards_without_the_probe() {
    let Some(built) = build("guard_fixture", false) else { return };
    let expected = tests_dir().join("guard_fixture/expected/out/shared/metadata.luau");
    let Ok(expected) = std::fs::read_to_string(expected) else { return };
    let ours = built.luau.get("shared/metadata.luau").expect("metadata.luau");
    assert_eq!(*ours, expected.replace("\r\n", "\n"));
}

/// The refused types: the TS builder's messages, at the TS's nodes, with its "Type was defined here" notes.
#[test]
fn unsupported_types_are_refused() {
    let Some(built) = build("guard_errors", true) else { return };
    let ours = built.diagnostics.join("\n") + "\n";
    let expected_path = tests_dir().join("guard_errors/expected-diagnostics.txt");
    if std::env::var_os("UPDATE_EXPECTED").is_some() {
        std::fs::write(&expected_path, &ours).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&expected_path).unwrap().replace("\r\n", "\n");
    assert!(ours == expected, "diagnostics differ\n--- ours\n{ours}");
}
