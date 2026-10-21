//! Project setup on the hand-made fixture projects (`tests/fixtures/setup/*`) against roblox-ts's own setup on them
//! (`tools/gen-setup-fixtures.cjs` wrote each `expected.json` with `tools/dump-project-services.cjs`). The answers are
//! win32 ones, so this runs on Windows.
//!
//! - `game-basic`: services, network types, an isolated container, ReplicatedFirst, an unsynced file, `init.client.ts`
//!   (checkFileName), an unsupported `rbxts` option (warning), `.tsx`.
//! - `rootdirs`: `rootDirs` without `rootDir` (TypeScript 5's computed common source directory) and `incremental`
//!   (build info path).
//! - `composite`: `composite` without `rootDir`, `tsBuildInfoFile`, `rbxts.luau: false` (`.lua` outputs), declarations.
//! - `package`: a scoped package without a Rojo file (synthetic resolver), `rbxts` options inherited through `extends`
//!   with a relative include path.
//! - `model`: a Rojo tree without `DataModel`.
//! - `invalid-options`, `rbxts-invalid`: setup errors.
//! - `no-rojo`, `rojo-in-src`, `isolated-runtime`: compileFiles's fatal failures and `checkRojoConfig`.
//! - `multi-rojo`: several project files (warning, first one used).
//! - `rbxts-rojo`: `rbxts.rojo` / `includePath` / `type` from the tsconfig.

#![cfg(windows)]

#[path = "../src/project_parity.rs"]
#[allow(dead_code)]
mod project_parity;

use rbxts_project::{STACK_SIZE, nodepath};

fn fixtures() -> String {
    nodepath::resolve(&[concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/setup")])
}

fn check(name: &'static str) {
    let root = fixtures();
    let path = format!("{root}\\{name}\\expected.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    // placeholders sit inside JSON strings, so the root goes in JSON-escaped
    let text = text.replace("{root_fwd}", &root.replace('\\', "/")).replace("{root}", &root.replace('\\', "\\\\"));
    let dump: serde_json::Value = serde_json::from_str(&text).unwrap();
    let report = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || project_parity::compare_dump(&dump, 50, 0))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(report.load_failures, 0, "{name}: tsrs could not load the project");
    assert!(report.compared > 0, "{name}: nothing compared");
    assert!(report.differences == 0, "{name}: {} differences:\n{}", report.differences, report.first.join("\n"));
}

#[test]
fn game_basic() {
    check("game-basic");
}

#[test]
fn rootdirs() {
    check("rootdirs");
}

#[test]
fn composite() {
    check("composite");
}

#[test]
fn package() {
    check("package");
}

#[test]
fn model() {
    check("model");
}

#[test]
fn invalid_options() {
    check("invalid-options");
}

#[test]
fn rbxts_invalid() {
    check("rbxts-invalid");
}

#[test]
fn no_rojo() {
    check("no-rojo");
}

#[test]
fn rojo_in_src() {
    check("rojo-in-src");
}

#[test]
fn isolated_runtime() {
    check("isolated-runtime");
}

#[test]
fn multi_rojo() {
    check("multi-rojo");
}

#[test]
fn rbxts_rojo() {
    check("rbxts-rojo");
}
