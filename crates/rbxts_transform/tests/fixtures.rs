//! The hand-written transform fixtures (`tests/fixtures/<area>`) against stock roblox-ts's output for them.
//!
//! A fixture is a roblox-ts project without `node_modules` (`src`, `tsconfig.json`, `default.project.json`, …) and
//! `expected/`, the Luau rbxts-rs must produce: stock roblox-ts's output (roblox-ts master through
//! `roblox-ts-test/probe/rbxts-driver.cjs`), except where a fixture's notes say otherwise. The test copies the
//! project (everything but `expected*/`) to a temporary directory, links in a `node_modules`, builds it and checks:
//! - every output equals its `expected/` file byte for byte, and every `expected/` file is produced;
//! - the build reports nothing: no diagnostic of any category (checker, setup or transform), no warning, no fatal
//!   error, no unsupported kind, no internal error. A fixture has no expected diagnostics. As with stock roblox-ts,
//!   diagnostics in declaration files under `src` are not reported, so a fixture can't test them.
//!
//! The shared `node_modules` (with `@rbxts/compiler-types`, `@rbxts/types`, `@rbxts/testez` and `@rbxts/services`) is
//! `RBXTS_RS_FIXTURE_NODE_MODULES` when set (a path that lacks the packages fails the test), else the roblox-ts
//! tests project's, found as `roblox-ts/tests/node_modules` or `roblox-ts-test/roblox-ts/tests/node_modules` under
//! the crate's directory or any directory above it. Without one the test is skipped and says so on the terminal
//! (outside the captured output), and passes, unless `RBXTS_RS_FIXTURE_REQUIRE=1` or `CI` is set: then it fails.
//!
//! A fixture's copy links the shared `node_modules` in as its `node_modules`, or, with a `links.json`, makes the
//! links that describes, in each of its layouts (see [`check`]). Linked `@rbxts/types` means no type counts as a
//! Roblox type (`is_in_rbxts_types`, as in stock roblox-ts), so the fixtures can't exercise the Roblox-type paths
//! (wrapping, the `instanceof` diagnostic).
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
//!   `node_modules`, with either the packages or the whole `node_modules` linked. Imports keep their `node_modules`
//!   paths, as with stock roblox-ts (`guessVirtualPath`).
//! - `junction_augmentation`: a module augmentation of a linked package, first in root order: its resolution counts
//!   like an import's, so another file's import through a second link to the package goes through the augmented one.
//! - `junction_interleaved_imports`, `junction_interleaved_type_refs`: a file's resolutions (imports, or
//!   `/// <reference types>`) all come before those of the files it reaches, so of two links to one package the one
//!   the file names wins over the one its dependency names.
//! - `junction_relative_dependency`: a package imported by relative path while also linked into `node_modules` as a
//!   dependency: TypeScript 5 learns links from resolutions only, so the import keeps the package's own path.
//! - `junction_two_links`: two links to one package: imports of both go through the one TypeScript 5 met first.
//! - `junction_workspace`: a bun / pnpm isolated workspace (`packages/game` and its sibling `packages/core`, both
//!   linking `@rbxts/services` from `node_modules/.store`): the first link met is game's own.
//!   `junction_workspace_core_first` imports `core` before `services`: game's link still comes first.
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

/// Fixtures must run (a missing `node_modules` fails them): `RBXTS_RS_FIXTURE_REQUIRE=1`, or `CI` set.
fn fixtures_required() -> bool {
    std::env::var_os("RBXTS_RS_FIXTURE_REQUIRE").is_some_and(|v| v == "1") || std::env::var_os("CI").is_some()
}

/// Skips a test without a `node_modules`, saying why on the terminal: written to stderr directly, since the test
/// harness captures (and, for a passing test, drops) `eprintln!`. A skipped test passes, unless
/// [`fixtures_required`].
fn skip(test: &str, why: &str) {
    assert!(!fixtures_required(), "fixtures::{test}: {why} (fixtures are required: CI or RBXTS_RS_FIXTURE_REQUIRE=1)");
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
    /// A copy of `fixture` without its `expected*/` folders and `links.json` (and any `node_modules` or `out`).
    fn copy_of(fixture: &Path, label: &str) -> TempProject {
        let root = std::env::temp_dir().join(format!("rbxts-rs-fixture-{label}-{}", std::process::id()));
        // a copy left by a killed run of the same process id
        let _ = std::fs::remove_dir_all(&root);
        let project = TempProject { root, links: Vec::new() };
        std::fs::create_dir_all(&project.root).unwrap();
        for entry in std::fs::read_dir(fixture).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("expected") || name == "links.json" || name == "node_modules" || name == "out" {
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
                checkers: None,
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

/// Builds a fixture copy, in each layout of its `links.json` (one default layout without one), and checks it.
///
/// `links.json` is `{ "project"?: dir, "layouts": { name: [[link, target], …] } }`: the project directory inside the
/// copy (default: the copy itself), and per layout the links to make, in order, each a path inside the copy and a
/// target, `@shared/<package>` for the shared `node_modules`'s `@rbxts/<package>` or a path inside the copy. Without
/// `links.json`, `node_modules` links to the shared `node_modules`.
fn check(area: &'static str) {
    let node_modules = match node_modules() {
        Ok(path) => path,
        Err(why) => return skip(area, &why),
    };
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(area);
    let links: Option<serde_json::Value> = std::fs::read_to_string(fixture.join("links.json"))
        .ok()
        .map(|text| serde_json::from_str(&text).unwrap_or_else(|e| panic!("{area}/links.json: {e}")));
    let project_dir = links.as_ref().and_then(|l| l["project"].as_str()).unwrap_or(".").to_owned();
    let layouts: Vec<(String, Vec<(String, String)>)> = match &links {
        None => vec![(String::new(), vec![("node_modules".to_owned(), node_modules.to_string_lossy().into_owned())])],
        Some(links) => links["layouts"]
            .as_object()
            .unwrap_or_else(|| panic!("{area}/links.json: no layouts"))
            .iter()
            .map(|(name, pairs)| {
                let pairs = pairs.as_array().unwrap_or_else(|| panic!("{area}/links.json: {name} is not a list"));
                let pairs = pairs
                    .iter()
                    .map(|pair| match pair.as_array().map(Vec::as_slice) {
                        Some([link, target]) => (link.as_str().unwrap().to_owned(), target.as_str().unwrap().to_owned()),
                        _ => panic!("{area}/links.json: {name}: a link is [link, target]"),
                    })
                    .collect();
                (name.clone(), pairs)
            })
            .collect(),
    };

    let mut failures = Vec::new();
    for (layout, pairs) in layouts {
        let label = if layout.is_empty() { area.to_owned() } else { format!("{area}-{layout}") };
        let mut project = TempProject::copy_of(&fixture, &label);
        for (link, target) in pairs {
            let target = match target.strip_prefix("@shared/") {
                Some(package) => node_modules.join("@rbxts").join(package),
                None if Path::new(&target).is_absolute() => PathBuf::from(target),
                None => project.root.join(target),
            };
            project.link(&link, &target);
        }
        let root = project.root.join(&project_dir);
        let report = build(&root);
        let layout_failures = compare(&report, &root, &fixture.join("expected"));
        if !layout_failures.is_empty() {
            failures.push(format!("{label}:\n{}", layout_failures.join("\n")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn junction() {
    check("junction");
}

#[test]
fn junction_augmentation() {
    check("junction_augmentation");
}

#[test]
fn junction_interleaved_imports() {
    check("junction_interleaved_imports");
}

#[test]
fn junction_interleaved_type_refs() {
    check("junction_interleaved_type_refs");
}

#[test]
fn junction_workspace_core_first() {
    check("junction_workspace_core_first");
}

#[test]
fn junction_relative_dependency() {
    check("junction_relative_dependency");
}

#[test]
fn junction_two_links() {
    check("junction_two_links");
}

#[test]
fn junction_workspace() {
    check("junction_workspace");
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
