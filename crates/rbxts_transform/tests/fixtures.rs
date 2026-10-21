//! The hand-written transform fixtures (`tests/fixtures/<area>`) against stock roblox-ts's output for them.
//!
//! A fixture is a roblox-ts project without `node_modules` (`src`, `tsconfig.json`, `default.project.json`, …) and
//! `expected/`, the Luau rbxts-rs must produce: stock roblox-ts's output (roblox-ts master through
//! `roblox-ts-test/probe/rbxts-driver.cjs`), except where a fixture's notes say otherwise. The test copies the
//! project (everything but `expected*/`) to a temporary directory, links in a `node_modules`, builds it and checks:
//! - every output equals its `expected/` file byte for byte, and every `expected/` file is produced;
//! - the build reports nothing: no diagnostic of any category (checker, setup or transform), no warning, no fatal
//!   error, no unsupported kind, no internal error. A fixture has no expected diagnostics.
//!
//! The `node_modules` (with `@rbxts/compiler-types`, `@rbxts/types`, `@rbxts/testez` and `@rbxts/services`) is
//! `RBXTS_RS_FIXTURE_NODE_MODULES` when set (a path that lacks the packages fails the test), else the roblox-ts
//! tests project's, found as `roblox-ts/tests/node_modules` or `roblox-ts-test/roblox-ts/tests/node_modules` under
//! the crate's directory or any directory above it. Without one the test is skipped and says so on the terminal
//! (outside the captured output).
//!
//! - `binding`: destructuring in declarations (`export let` and hoisted names included), parameters (rest tuples
//!   too) and assignments (optimized, nested, below a rest element, used as values), over arrays, tuples, LuaTuple
//!   calls, strings, sets, maps, iterable functions, generators and `SharedTable`; computed, string and numeric keys
//!   on strings and arrays; patterns in class constructors and methods.
//! - `classes`: class declarations and expressions, constructors (parameter properties, property initializers,
//!   `super` calls), methods with unusual names, static members and blocks, `this` in static members, hoisting,
//!   default exports, and every kind of decorator. TypeScript 5 lists exported functions first and TypeScript 7 in
//!   source order, so the fixture declares its exported functions first.
//! - `contextual_this`: function expressions and object literal methods in slots whose signature has a `this`
//!   parameter (TypeScript 5 gives them its contextual `this`), next to arrows, `this` users and `this: void` slots.
//! - `exprs`: conditional expressions, `||=` / `&&=` / `??=`, `yield` and `yield*`, `delete`, `await`, tagged
//!   templates, `import()`, mixed edge cases, and a server script importing the modules.
//! - `jsx`: JSX elements, attributes (string, boolean, namespaced, with prerequisites), spread props (first, later,
//!   possibly undefined, conditional), fragments, keyed, conditional, mapped and spread children, JSX text whitespace
//!   and entities, `@jsx` / `@jsxFrag` pragmas, and the JSX factory import kept alive when only JSX uses it.
//! - `junction`: static and dynamic imports of packages behind junctions (symlinks elsewhere): `@rbxts/services`
//!   in the shared `node_modules`, and `@rbxts/greeter` (the fixture's `packages/greeter`) outside any
//!   `node_modules`, with either the packages or the whole `node_modules` linked (see `check_junction`). Imports keep
//!   their `node_modules` paths, as with stock roblox-ts (`guessVirtualPath`).
//! - `literals`: object and array literals, methods in object literals, spread in objects, arrays and calls, every
//!   iterable kind an array spread accepts. TypeScript 5 lists exported functions first and TypeScript 7 in source
//!   order, so the fixture declares its exported functions first.
//! - `loops`: numeric `for` loops under `--optimizedLoops` and the `while` fallback (copies of captured loop
//!   variables, finalizers before `continue`), every for-of loop builder and `$range`, `while`, `do`-`while`, and
//!   `continue` / `break` inside `try` in loops, and roblox-ts's `loop.spec.ts` (destructured loop variables included).
//! - `macros`: every call, identifier, constructor and property-call macro, as values and statements, on optional
//!   chains and with evaluation-order cases.
//! - `stmts`: enums (non-finite values too), namespaces, `switch`, and `try` / `throw` (roblox-ts tests' specs plus
//!   hand-written cases). `expected/` is roblox-ts on the TypeScript 7 checker, except the infinities, which rbxts-rs
//!   writes as `math.huge`; `expected-ts5/` (not checked) keeps the two files where stock roblox-ts on TypeScript 5
//!   differs, in the order of the export table. Its tsconfig turns on `allowCommentDirectives` (for
//!   `@ts-expect-error` in enums).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use rbxts_project::{ProjectOptionLayer, STACK_SIZE};
use rbxts_transform::{CompileOptions, CompileReport, compile_project};
use tsrs_compiler::diagnosticwriter::flatten_diagnostic_message;
use tsrs_diagnostics::Category;

const NODE_MODULES_VAR: &str = "RBXTS_RS_FIXTURE_NODE_MODULES";
/// The packages a fixture's `node_modules` must have.
const PACKAGES: [&str; 4] = ["compiler-types", "types", "testez", "services"];

fn has_packages(node_modules: &Path) -> bool {
    PACKAGES.iter().all(|name| node_modules.join("@rbxts").join(name).is_dir())
}

/// The `node_modules` to link into fixtures, or `None` (with the reason) to skip. Panics when
/// `RBXTS_RS_FIXTURE_NODE_MODULES` names a directory without the packages: a set variable means the fixtures should
/// run.
fn node_modules() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os(NODE_MODULES_VAR) {
        let path = PathBuf::from(path);
        assert!(
            has_packages(&path),
            "{NODE_MODULES_VAR}={} has no @rbxts/{{{}}}",
            path.display(),
            PACKAGES.join(", ")
        );
        return Ok(path);
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .flat_map(|dir| {
            [dir.join("roblox-ts/tests/node_modules"), dir.join("roblox-ts-test/roblox-ts/tests/node_modules")]
        })
        .find(|candidate| has_packages(candidate))
        .ok_or_else(|| {
            format!(
                "no node_modules with @rbxts/{{{}}}: set {NODE_MODULES_VAR}, or have the roblox-ts tests project \
                 (roblox-ts/tests or roblox-ts-test/roblox-ts/tests) with its packages installed beside {} or a \
                 directory above it",
                PACKAGES.join(", "),
                manifest.display()
            )
        })
}

/// Says why a test is skipped on the terminal: written to stderr directly, since the test harness captures (and,
/// for a passing test, drops) `eprintln!`.
fn announce_skip(test: &str, why: &str) {
    let _ = writeln!(std::io::stderr(), "fixtures::{test} skipped: {why}");
}

/// Links `link` to the directory `target`: a junction on Windows (it needs no privileges, unlike a directory
/// symlink), a symlink elsewhere.
fn link_dir(link: &Path, target: &Path) {
    #[cfg(windows)]
    {
        // mklink wants backslashes
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(target.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "could not link {} to {}: {}",
            link.display(),
            target.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[cfg(not(windows))]
    std::os::unix::fs::symlink(target, link)
        .unwrap_or_else(|e| panic!("could not link {} to {}: {e}", link.display(), target.display()));
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

/// A fixture's temporary copy, removed on drop, panics included. Links inside it are removed as links first, never
/// followed into the shared `node_modules`.
struct TempProject {
    root: PathBuf,
    links: Vec<PathBuf>,
}

impl TempProject {
    /// A copy of `fixture` without its `expected*/` folders (and any `node_modules` or `out`).
    fn copy_of(fixture: &Path, label: &str) -> TempProject {
        let root = std::env::temp_dir().join(format!("rbxts-rs-fixture-{label}-{}", std::process::id()));
        // a copy left by a killed run of the same process id
        let _ = std::fs::remove_dir_all(&root);
        let project = TempProject { root, links: Vec::new() };
        std::fs::create_dir_all(&project.root).unwrap();
        for entry in std::fs::read_dir(fixture).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("expected") || name == "node_modules" || name == "out" {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &project.root.join(&name));
            } else {
                std::fs::copy(entry.path(), project.root.join(&name)).unwrap();
            }
        }
        project
    }

    /// Links `relative` (inside the copy) to `target`.
    fn link(&mut self, relative: &str, target: &Path) {
        let link = self.root.join(relative);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        link_dir(&link, target);
        self.links.push(link);
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        // innermost links first; removing a junction or a directory symlink removes the link, not its target
        for link in self.links.iter().rev() {
            if std::fs::remove_dir(link).is_err() {
                let _ = std::fs::remove_file(link);
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Builds the project at `root` (no writes) on a thread with the compiler's stack size.
fn build(root: &Path) -> CompileReport {
    let cwd = root.to_string_lossy().into_owned();
    std::thread::Builder::new()
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
        .unwrap()
}

/// Everything the build reported, one line each: fatal errors, warnings, diagnostics, internal errors and
/// unsupported kinds.
fn reported(report: &CompileReport, root: &Path) -> Vec<String> {
    let relative = |file: &str| {
        Path::new(file).strip_prefix(root).map_or_else(|_| file.to_owned(), |p| p.to_string_lossy().replace('\\', "/"))
    };
    let mut lines: Vec<String> = report.fatal.iter().map(|m| format!("fatal: {m}")).collect();
    lines.extend(report.warnings.iter().map(|w| format!("warning: {w}")));
    let all_diagnostics = report.diagnostics.iter().chain(report.files.iter().flat_map(|f| f.diagnostics.iter()));
    for d in all_diagnostics {
        let category = match d.category() {
            Category::Error => "error",
            Category::Warning => "warning",
            Category::Suggestion => "suggestion",
            Category::Message => "message",
        };
        let file = d.file().map(|f| relative(f.file_name())).unwrap_or_default();
        lines.push(format!("{category} {file}: {}", flatten_diagnostic_message(*d, " ")));
    }
    for file in &report.files {
        let name = relative(&file.file_name);
        lines.extend(file.internal_errors.iter().map(|e| format!("internal error {name}: {e}")));
        if !file.unsupported.is_empty() {
            lines.push(format!("unsupported {name}: {:?}", file.unsupported));
        }
    }
    lines
}

/// The build of `root` against `expected`: every failure, one entry each.
fn compare(report: &CompileReport, root: &Path, expected: &Path) -> Vec<String> {
    let mut failures = reported(report, root);
    let mut produced = Vec::new();
    for file in &report.files {
        let relative = Path::new(&file.out_path)
            .strip_prefix(&report.out_dir)
            .unwrap_or(Path::new(&file.out_path))
            .to_string_lossy()
            .replace('\\', "/");
        produced.push(relative.clone());
        let Some(luau) = &file.luau else {
            failures.push(format!("{relative}: no output"));
            continue;
        };
        let expected_path = expected.join(&relative);
        let Ok(expected) = std::fs::read_to_string(&expected_path) else {
            failures.push(format!("{relative}: produced but not in expected/\n--- ours\n{luau}"));
            continue;
        };
        let expected = expected.replace("\r\n", "\n");
        if *luau != expected {
            let line = luau
                .lines()
                .zip(expected.lines())
                .position(|(a, b)| a != b)
                .map_or_else(|| luau.lines().count().min(expected.lines().count()) + 1, |i| i + 1);
            failures.push(format!("{relative}: differs from expected at line {line}\n--- ours\n{luau}"));
        }
    }
    let mut expected_files = Vec::new();
    collect_files(expected, expected, &mut expected_files);
    for relative in expected_files {
        if !produced.contains(&relative) {
            failures.push(format!("{relative}: expected but not produced"));
        }
    }
    failures
}

fn check(area: &'static str) {
    let node_modules = match node_modules() {
        Ok(path) => path,
        Err(why) => return announce_skip(area, &why),
    };
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(area);
    let mut project = TempProject::copy_of(&fixture, area);
    project.link("node_modules", &node_modules);
    let report = build(&project.root);
    let failures = compare(&report, &project.root, &fixture.join("expected"));
    assert!(failures.is_empty(), "{area}:\n{}", failures.join("\n"));
}

/// How the `junction` fixture's packages are linked in.
#[derive(Clone, Copy)]
enum Layout {
    /// `node_modules/@rbxts` is a real directory whose packages are links: the shared `node_modules`'s, and
    /// `greeter` to the copy's `packages/greeter`.
    LinkedPackages,
    /// `node_modules` links to `store/node_modules`, laid out as in `LinkedPackages`.
    LinkedNodeModules,
}

/// The `junction` fixture in `layout`. Module resolution gives every package its real path (in the shared
/// `node_modules`, or `packages/greeter` outside any `node_modules`); imports must still go through the project's
/// `node_modules`, as stock roblox-ts's do in both layouts (`expected/` is its output, the same for both).
fn check_junction(layout: Layout) {
    const AREA: &str = "junction";
    let label = match layout {
        Layout::LinkedPackages => "junction-packages",
        Layout::LinkedNodeModules => "junction-node-modules",
    };
    let node_modules = match node_modules() {
        Ok(path) => path,
        Err(why) => return announce_skip(label, &why),
    };
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(AREA);
    let mut project = TempProject::copy_of(&fixture, label);
    let scope = match layout {
        Layout::LinkedPackages => "node_modules/@rbxts",
        Layout::LinkedNodeModules => "store/node_modules/@rbxts",
    };
    for name in PACKAGES {
        project.link(&format!("{scope}/{name}"), &node_modules.join("@rbxts").join(name));
    }
    let greeter = project.root.join("packages/greeter");
    project.link(&format!("{scope}/greeter"), &greeter);
    if let Layout::LinkedNodeModules = layout {
        let store = project.root.join("store/node_modules");
        project.link("node_modules", &store);
    }
    let report = build(&project.root);
    let failures = compare(&report, &project.root, &fixture.join("expected"));
    assert!(failures.is_empty(), "{label}:\n{}", failures.join("\n"));
}

#[test]
fn junction_linked_packages() {
    check_junction(Layout::LinkedPackages);
}

#[test]
fn junction_linked_node_modules() {
    check_junction(Layout::LinkedNodeModules);
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
fn contextual_this() {
    check("contextual_this");
}

#[test]
fn exprs() {
    check("exprs");
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

#[test]
fn stmts() {
    check("stmts");
}
