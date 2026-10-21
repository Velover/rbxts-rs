//! The hand-written transform fixtures (`tests/fixtures/<area>`) against stock roblox-ts's output for them
//! (`expected/`, written by roblox-ts master on TypeScript 5 through `roblox-ts-test/probe/rbxts-driver.cjs
//! --backend ts5`).
//!
//! A fixture is a roblox-ts project without `node_modules`. The test copies it to a temporary directory and links
//! in a `node_modules` with `@rbxts/compiler-types` and `@rbxts/types`: `RBXTS_RS_FIXTURE_NODE_MODULES`, else the
//! roblox-ts tests project's. Without one the test is skipped (it says so on stderr).
//!
//! - `binding`: destructuring in declarations (`export let` and hoisted names included), parameters (rest tuples
//!   too) and assignments (optimized, nested, below a rest element, used as values), over arrays, tuples, LuaTuple
//!   calls, strings, sets, maps, iterable functions, generators and `SharedTable`.
//! - `classes`: class declarations and expressions, constructors (parameter properties, property initializers,
//!   `super` calls), methods with unusual names, static members and blocks, `this` in static members, hoisting,
//!   default exports, and every kind of decorator. TypeScript 5 lists exported functions first and TypeScript 7 in
//!   source order, so the fixture declares its exported functions first.
//! - `jsx`: JSX elements, attributes (string, boolean, namespaced, with prerequisites), spread props (first, later,
//!   possibly undefined, conditional), fragments, keyed, conditional, mapped and spread children, JSX text whitespace
//!   and entities, `@jsx` / `@jsxFrag` pragmas, and the JSX factory import kept alive when only JSX uses it.
//! - `literals`: object and array literals, methods in object literals, spread in objects, arrays and calls, every
//!   iterable kind an array spread accepts. TypeScript 5 lists exported functions first and TypeScript 7 in source
//!   order, so the fixture declares its exported functions first.
//! - `loops`: numeric `for` loops under `--optimizedLoops` and the `while` fallback (copies of captured loop
//!   variables, finalizers before `continue`), every for-of loop builder and `$range`, `while`, `do`-`while`, and
//!   `continue` / `break` inside `try` in loops.
//! - `macros`: every call, identifier, constructor and property-call macro, as values and statements, on optional
//!   chains and with evaluation-order cases.

#![cfg(windows)]

use std::path::{Path, PathBuf};

use rbxts_project::{ProjectOptionLayer, STACK_SIZE};
use rbxts_transform::{CompileOptions, compile_project};

const DEFAULT_NODE_MODULES: &str = "E:/Projects/TS/Flamework/roblox-ts-test/roblox-ts/tests/node_modules";

fn node_modules() -> Option<PathBuf> {
    let path = std::env::var_os("RBXTS_RS_FIXTURE_NODE_MODULES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_NODE_MODULES));
    path.join("@rbxts").join("compiler-types").is_dir().then_some(path)
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

fn collect_files(base: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(base, &path, out);
        } else {
            out.push(path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
}

fn check(area: &'static str) {
    let Some(node_modules) = node_modules() else {
        eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
        return;
    };
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(area);
    let root = std::env::temp_dir().join(format!("rbxts-rs-fixture-{area}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for name in ["src", "tsconfig.json", "default.project.json", "package.json"] {
        let from = fixture.join(name);
        if from.is_dir() {
            copy_dir(&from, &root.join(name));
        } else {
            std::fs::copy(&from, root.join(name)).unwrap();
        }
    }
    // a junction needs no privileges, unlike a directory symlink; mklink wants backslashes
    let link = root.join("node_modules");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(node_modules.to_string_lossy().replace('/', "\\"))
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "could not link node_modules");

    let cwd = root.to_string_lossy().into_owned();
    let report = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            compile_project(&CompileOptions {
                tsconfig: "tsconfig.json".into(),
                cwd,
                project_options: ProjectOptionLayer::default(),
                out_dir_override: None,
                write_partial: false,
                no_write: true,
            })
        })
        .unwrap()
        .join()
        .unwrap();

    let mut failures = Vec::new();
    failures.extend(report.fatal.iter().cloned());
    let mut produced = Vec::new();
    for file in &report.files {
        let relative = Path::new(&file.out_path)
            .strip_prefix(&report.out_dir)
            .unwrap_or(Path::new(&file.out_path))
            .to_string_lossy()
            .replace('\\', "/");
        produced.push(relative.clone());
        if !file.unsupported.is_empty() {
            failures.push(format!("{relative}: unsupported {:?}", file.unsupported));
            continue;
        }
        let Some(luau) = &file.luau else {
            failures.push(format!("{relative}: no output ({} diagnostics)", file.diagnostics.len()));
            continue;
        };
        let expected_path = fixture.join("expected").join(&relative);
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("{}: {e}", expected_path.display()))
            .replace("\r\n", "\n");
        if *luau != expected {
            let line = luau.lines().zip(expected.lines()).position(|(a, b)| a != b).map_or(0, |i| i + 1);
            failures.push(format!("{relative}: differs from expected at line {line}\n--- ours\n{luau}"));
        }
    }
    let mut expected_files = Vec::new();
    collect_files(&fixture.join("expected"), &fixture.join("expected"), &mut expected_files);
    for relative in expected_files {
        if !produced.contains(&relative) {
            failures.push(format!("{relative}: expected but not produced"));
        }
    }

    let _ = std::process::Command::new("cmd").args(["/C", "rmdir"]).arg(&link).status();
    let _ = std::fs::remove_dir_all(&root);
    assert!(failures.is_empty(), "{area}:\n{}", failures.join("\n"));
}

#[test]
fn binding() {
    check("binding");
}

#[test]
fn classes() {
    check("classes");
}

#[test]
fn jsx() {
    check("jsx");
}

#[test]
fn literals() {
    check("literals");
}

#[test]
fn loops() {
    check("loops");
}

#[test]
fn macros() {
    check("macros");
}
