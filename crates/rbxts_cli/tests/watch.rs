//! `rbxtsc -w --events json` as a process on a copy of `rbxts_transform`'s watch fixture: the real watcher's
//! debounce (a burst of writes is one rebuild), a rename (the old output goes, the new one comes), an error (nothing
//! written) and its fix. The finer invariants are the in-process harness's (`rbxts_transform/tests/watch.rs`).

#![cfg(windows)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use serde_json::Value;

const DEFAULT_NODE_MODULES: &str = "E:/Projects/TS/Flamework/roblox-ts-test/roblox-ts/tests/node_modules";

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

/// The watcher process and its copy, both gone on drop.
struct Watcher {
    child: Child,
    root: PathBuf,
    events: Receiver<Value>,
    /// Every stdout line, for failure messages.
    lines: Receiver<String>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let link = self.root.join("node_modules");
        let _ = Command::new("cmd")
            .args(["/C", "rmdir"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .stderr(Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Watcher {
    /// The next `build-end` event (the `build-start` events are checked on the way: one per generation).
    fn build_end(&self, expected_gen: u64) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let event = match self.events.recv_timeout(left) {
                Ok(event) => event,
                Err(_) => {
                    let output: Vec<String> = self.lines.try_iter().collect();
                    panic!("no build-end for generation {expected_gen} within 60 s; output:\n{}", output.join("\n"));
                }
            };
            assert_eq!(event["gen"].as_u64(), Some(expected_gen), "unexpected event {event}");
            if event["event"] == "build-end" {
                return event;
            }
        }
    }

    /// No further build starts within `quiet`.
    fn assert_quiet(&self, quiet: Duration) {
        if let Ok(event) = self.events.recv_timeout(quiet) {
            panic!("unexpected event {event}");
        }
    }
}

#[test]
fn the_watcher_debounces_renames_and_recovers_from_errors() {
    let node_modules = std::env::var_os("RBXTS_RS_FIXTURE_NODE_MODULES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_NODE_MODULES));
    if !node_modules.join("@rbxts").join("compiler-types").is_dir() {
        eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
        return;
    }
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../rbxts_transform/tests/watch_fixtures/basic");
    let root = std::env::temp_dir().join(format!("rbxts-rs-cli-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    copy_dir(&fixture, &root);
    // the plain command line has no stamp plugin
    let tsconfig = std::fs::read_to_string(root.join("tsconfig.json")).unwrap();
    let tsconfig = tsconfig.replace(
        "\"plugins\": [{ \"transform\": \"rbxts-transform-stamp\", \"prefix\": \"watch \" }]",
        "\"plugins\": []",
    );
    std::fs::write(root.join("tsconfig.json"), tsconfig).unwrap();
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(root.join("node_modules").to_string_lossy().replace('/', "\\"))
        .arg(node_modules.to_string_lossy().replace('/', "\\"))
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "could not link node_modules");

    let mut child = Command::new(env!("CARGO_BIN_EXE_rbxtsc"))
        .args(["-w", "--events", "json", "-p"])
        .arg(&root)
        .env("RBXTS_WATCH_MEMORY_LIMIT_MB", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (event_tx, events) = channel();
    let (line_tx, lines) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.starts_with("{\"event\"")
                && let Ok(value) = serde_json::from_str::<Value>(&line)
            {
                let _ = event_tx.send(value);
            }
            let _ = line_tx.send(line);
        }
    });
    let watcher = Watcher { child, root: root.clone(), events, lines };
    let initial = watcher.build_end(0);
    assert_eq!(initial["ok"], true, "{initial}");
    assert!(root.join("out/shared/d.luau").is_file());

    // a burst of three writes inside the debounce: one rebuild
    let c = root.join("src/shared/c.ts");
    for text in ["Map<string, number>", "Map<string, boolean>", "Map<string, string>"] {
        std::fs::write(&c, format!("export interface Inner {{\n\ta: {text};\n}}\n\nexport const cValue = 1;\n"))
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    let end = watcher.build_end(1);
    assert_eq!(end["ok"], true, "{end}");
    let d = std::fs::read_to_string(root.join("out/shared/d.luau")).unwrap();
    assert!(d.contains("ReadonlyMap.size"), "D was rebuilt with Inner.a a map:\n{d}");
    watcher.assert_quiet(Duration::from_millis(500));

    // a rename: the old output goes, the new one comes
    std::fs::rename(root.join("src/shared/importer.ts"), root.join("src/shared/importer2.ts")).unwrap();
    let end = watcher.build_end(2);
    assert_eq!(end["ok"], true, "{end}");
    assert!(!root.join("out/shared/importer.luau").exists());
    assert!(root.join("out/shared/importer2.luau").is_file());

    // an error: nothing written; the fix
    let before = std::fs::read_to_string(root.join("out/shared/d.luau")).unwrap();
    std::fs::write(
        root.join("src/shared/d.ts"),
        "import { A } from \"./b\";\n\nexport function sizeOf(value: A) {\n\treturn value.x.a.sizee();\n}\n",
    )
    .unwrap();
    let end = watcher.build_end(3);
    assert_eq!(end["ok"], false, "{end}");
    assert_eq!(std::fs::read_to_string(root.join("out/shared/d.luau")).unwrap(), before);
    std::fs::write(
        root.join("src/shared/d.ts"),
        "import { A } from \"./b\";\n\nexport function sizeOf(value: A) {\n\treturn value.x.a.size() + 1;\n}\n",
    )
    .unwrap();
    let end = watcher.build_end(4);
    assert_eq!(end["ok"], true, "{end}");
    assert!(std::fs::read_to_string(root.join("out/shared/d.luau")).unwrap().contains("+ 1"));
    drop(watcher);
}

/// Unlinks `link` (a junction) without touching its target.
fn unlink(link: &Path) {
    let _ = Command::new("cmd").args(["/C", "rmdir"]).arg(link.to_string_lossy().replace('/', "\\")).status();
}

/// Starts `rbxtsc -w --events json` on `root` and waits for its initial build.
fn start(root: &Path) -> Watcher {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rbxtsc"))
        .args(["-w", "--events", "json", "-p"])
        .arg(root)
        .env("RBXTS_WATCH_MEMORY_LIMIT_MB", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (event_tx, events) = channel();
    let (line_tx, lines) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.starts_with("{\"event\"")
                && let Ok(value) = serde_json::from_str::<Value>(&line)
            {
                let _ = event_tx.send(value);
            }
            let _ = line_tx.send(line);
        }
    });
    Watcher { child, root: root.to_path_buf(), events, lines }
}

impl Watcher {
    /// Takes events until none comes for 1.5 s; the last build-end seen.
    fn settle(&self) -> Option<Value> {
        let mut last = None;
        while let Ok(event) = self.events.recv_timeout(Duration::from_millis(1500)) {
            if event["event"] == "build-end" {
                last = Some(event);
            }
        }
        last
    }

    /// Build ends until one that succeeded (a package written in steps can take more than one rebuild), within 60 s.
    fn until_ok(&self) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.events.recv_timeout(left) {
                Ok(event) if event["event"] == "build-end" && event["ok"] == true => return event,
                Ok(_) => {}
                Err(_) => {
                    let output: Vec<String> = self.lines.try_iter().collect();
                    panic!("no successful build within 60 s; output:\n{}", output.join("\n"));
                }
            }
        }
    }
}

/// Events from outside the sources: a directory moved into `src` from outside the watched roots (one event for the
/// directory on Windows), and a package installed, then changed, under `node_modules` with no source edit.
#[test]
fn the_watcher_sees_directories_moved_in_and_packages_installed() {
    let node_modules = std::env::var_os("RBXTS_RS_FIXTURE_NODE_MODULES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_NODE_MODULES));
    if !node_modules.join("@rbxts").join("compiler-types").is_dir() {
        eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
        return;
    }
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../rbxts_transform/tests/watch_fixtures/basic");
    let base = std::env::temp_dir().join(format!("rbxts-rs-cli-watch-pkg-{}", std::process::id()));
    let root = base.join("project");
    let outside = base.join("outside");
    let _ = std::fs::remove_dir_all(&base);
    copy_dir(&fixture, &root);
    let tsconfig = std::fs::read_to_string(root.join("tsconfig.json")).unwrap();
    let tsconfig = tsconfig.replace(
        "\"plugins\": [{ \"transform\": \"rbxts-transform-stamp\", \"prefix\": \"watch \" }]",
        "\"plugins\": []",
    );
    std::fs::write(root.join("tsconfig.json"), tsconfig).unwrap();
    // a real node_modules and @rbxts, one junction per package, so a package can be added in the copy alone
    let scope = root.join("node_modules/@rbxts");
    std::fs::create_dir_all(&scope).unwrap();
    let mut links = Vec::new();
    for entry in std::fs::read_dir(node_modules.join("@rbxts")).unwrap().flatten() {
        let link = scope.join(entry.file_name());
        let status = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(entry.path().to_string_lossy().replace('/', "\\"))
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        links.push(link);
    }
    std::fs::create_dir_all(outside.join("feat")).unwrap();
    std::fs::write(outside.join("feat/f.ts"), "export const f = 1;\n").unwrap();

    let watcher = start(&root);
    let initial = watcher.build_end(0);
    assert_eq!(initial["ok"], true, "{initial}");

    // a directory moved in from outside the watched roots
    std::fs::rename(outside.join("feat"), root.join("src/shared/feat")).unwrap();
    let end = watcher.build_end(1);
    assert_eq!(end["ok"], true, "{end}");
    assert!(root.join("out/shared/feat/f.luau").is_file(), "the moved-in directory was built");

    // an import of a package not installed yet
    std::fs::write(
        root.join("src/shared/usePkg.ts"),
        "import { v } from \"@rbxts/vpkg\";\n\nexport const w = v.size();\n",
    )
    .unwrap();
    let end = watcher.build_end(2);
    assert_eq!(end["ok"], false, "{end}");

    // the package appears (no source edit): a rebuild resolves it
    let staged = base.join("vpkg");
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::write(
        staged.join("package.json"),
        "{ \"name\": \"@rbxts/vpkg\", \"main\": \"init.lua\", \"types\": \"index.d.ts\" }\n",
    )
    .unwrap();
    std::fs::write(staged.join("index.d.ts"), "export declare const v: Array<number>;\n").unwrap();
    std::fs::write(staged.join("init.lua"), "return { v = {} }\n").unwrap();
    std::fs::rename(&staged, scope.join("vpkg")).unwrap();
    watcher.until_ok();
    let use_pkg = std::fs::read_to_string(root.join("out/shared/usePkg.luau")).unwrap();
    assert!(!use_pkg.contains("ReadonlyMap.size"), "{use_pkg}");

    // the package's declaration changes (no source edit): a rebuild follows it
    std::fs::write(scope.join("vpkg/index.d.ts"), "export declare const v: Map<number, number>;\n").unwrap();
    watcher.until_ok();
    let use_pkg = std::fs::read_to_string(root.join("out/shared/usePkg.luau")).unwrap();
    assert!(use_pkg.contains("ReadonlyMap.size"), "the package's new types were used:\n{use_pkg}");

    for link in links {
        unlink(&link);
    }
    drop(watcher);
    let _ = std::fs::remove_dir_all(&base);
}

/// A copy of the watch fixture without the stamp plugin, with a real `node_modules` and `@rbxts` holding one
/// junction per package (so packages can be added in the copy alone). Returns (base, project, junctions).
fn linked_project(label: &str) -> Option<(PathBuf, PathBuf, Vec<PathBuf>)> {
    let node_modules = std::env::var_os("RBXTS_RS_FIXTURE_NODE_MODULES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_NODE_MODULES));
    if !node_modules.join("@rbxts").join("compiler-types").is_dir() {
        eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
        return None;
    }
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../rbxts_transform/tests/watch_fixtures/basic");
    let base = std::env::temp_dir().join(format!("rbxts-rs-cli-watch-{label}-{}", std::process::id()));
    let root = base.join("project");
    let _ = std::fs::remove_dir_all(&base);
    copy_dir(&fixture, &root);
    let tsconfig = std::fs::read_to_string(root.join("tsconfig.json")).unwrap().replace(
        "\"plugins\": [{ \"transform\": \"rbxts-transform-stamp\", \"prefix\": \"watch \" }]",
        "\"plugins\": []",
    );
    std::fs::write(root.join("tsconfig.json"), tsconfig).unwrap();
    let links = link_packages(&root.join("node_modules/@rbxts"));
    Some((base, root, links))
}

/// The fixture's `node_modules`: `RBXTS_RS_FIXTURE_NODE_MODULES` or roblox-ts's tests project's.
fn fixture_node_modules() -> PathBuf {
    std::env::var_os("RBXTS_RS_FIXTURE_NODE_MODULES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_NODE_MODULES))
}

/// Makes `scope` (a real directory) with one junction per package of the fixture's `@rbxts`; the junctions.
fn link_packages(scope: &Path) -> Vec<PathBuf> {
    std::fs::create_dir_all(scope).unwrap();
    let mut links = Vec::new();
    for entry in std::fs::read_dir(fixture_node_modules().join("@rbxts")).unwrap().flatten() {
        let link = scope.join(entry.file_name());
        let status = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(entry.path().to_string_lossy().replace('/', "\\"))
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        links.push(link);
    }
    links
}

/// A package folder created (one rebuild starts), its files written while that rebuild runs: the folder is
/// watched only after the build, so the writes raise no event of their own, yet the watcher ends on a successful
/// build. Then churn under node_modules (`.package-lock.json`, `.cache/`) starts no rebuild.
#[test]
fn files_written_into_a_new_package_folder_during_its_rebuild_are_seen() {
    let Some((base, root, links)) = linked_project("late-pkg") else { return };
    std::fs::write(
        root.join("src/shared/usePkg.ts"),
        "import { v } from \"@rbxts/latepkg\";\n\nexport const w = v.size();\n",
    )
    .unwrap();
    let watcher = start(&root);
    let initial = watcher.build_end(0);
    assert_eq!(initial["ok"], false, "the package is missing: {initial}");

    let package = root.join("node_modules/@rbxts/latepkg");
    std::fs::create_dir(&package).unwrap();
    // wait for the rebuild the folder starts, then fill the folder while it runs
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let event = watcher.events.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())).unwrap();
        if event["event"] == "build-start" {
            break;
        }
    }
    std::fs::write(
        package.join("package.json"),
        "{ \"name\": \"@rbxts/latepkg\", \"main\": \"init.lua\", \"types\": \"index.d.ts\" }\n",
    )
    .unwrap();
    std::fs::write(package.join("index.d.ts"), "export declare const v: Array<number>;\n").unwrap();
    std::fs::write(package.join("init.lua"), "return { v = {} }\n").unwrap();
    watcher.until_ok();
    // a write the build may not have seen starts one more rebuild (the folder is compared with the build's start)
    let last = watcher.settle();
    assert!(last.as_ref().is_none_or(|end| end["ok"] == true), "the last build failed: {last:?}");
    assert!(root.join("out/shared/usePkg.luau").is_file());

    // node_modules churn: dot entries are no packages
    std::fs::write(root.join("node_modules/.package-lock.json"), "{}\n").unwrap();
    std::fs::create_dir_all(root.join("node_modules/.cache/x")).unwrap();
    std::fs::write(root.join("node_modules/.cache/x/y"), "z").unwrap();
    std::fs::write(root.join("node_modules/@rbxts/.lock"), "z").unwrap();
    watcher.assert_quiet(Duration::from_millis(1500));

    for link in links {
        unlink(&link);
    }
    drop(watcher);
    let _ = std::fs::remove_dir_all(&base);
}

/// A tsconfig whose outDir is the project directory fails the build; the watcher must still see the tsconfig
/// restored (its events are configs, never ignored as outputs) and later source edits.
#[test]
fn the_watcher_hears_the_tsconfig_after_an_out_dir_holding_the_project() {
    let Some((base, root, links)) = linked_project("outdir") else { return };
    let watcher = start(&root);
    let initial = watcher.build_end(0);
    assert_eq!(initial["ok"], true, "{initial}");
    let tsconfig = std::fs::read_to_string(root.join("tsconfig.json")).unwrap();
    std::fs::write(root.join("tsconfig.json"), tsconfig.replace("\"outDir\": \"out\"", "\"outDir\": \".\"")).unwrap();
    let end = watcher.build_end(1);
    assert_eq!(end["ok"], false, "an outDir holding the sources fails: {end}");
    std::fs::write(root.join("tsconfig.json"), &tsconfig).unwrap();
    let end = watcher.build_end(2);
    assert_eq!(end["ok"], true, "the restored tsconfig rebuilt: {end}");
    std::fs::write(root.join("src/shared/later.ts"), "export const later = 1;\n").unwrap();
    let end = watcher.build_end(3);
    assert_eq!(end["ok"], true, "{end}");
    assert!(root.join("out/shared/later.luau").is_file());
    for link in links {
        unlink(&link);
    }
    drop(watcher);
    let _ = std::fs::remove_dir_all(&base);
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Sets a file's modification time to 1985, as an archive extraction or `cp -p` of an old tree leaves it.
fn set_old(path: &Path) {
    let old = std::time::UNIX_EPOCH + Duration::from_secs(499_162_500);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(old).unwrap();
}

/// Removes a directory tree, retrying while a watch handle holds a directory in it pending deletion.
fn remove_tree(path: &Path) {
    for _ in 0..50 {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    panic!("could not remove {}", path.display());
}

impl Watcher {
    /// Waits for the next build to start (within 60 s).
    fn build_start(&self) {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.events.recv_timeout(left) {
                Ok(event) if event["event"] == "build-start" => return,
                Ok(_) => {}
                Err(_) => panic!("no build started within 60 s"),
            }
        }
    }

    /// Settles (no build for 1.5 s) and checks that the last build succeeded.
    fn settled_ok(&self, label: &str) {
        let last = self.settle();
        assert!(last.as_ref().is_some_and(|end| end["ok"] == true), "{label}: the last build failed: {last:?}");
    }

    /// After a successful build: settles, and checks that any build since succeeded too.
    fn still_ok(&self, label: &str) {
        let last = self.settle();
        assert!(last.as_ref().is_none_or(|end| end["ok"] == true), "{label}: the last build failed: {last:?}");
    }
}

fn package_json(name: &str) -> String {
    format!("{{ \"name\": \"@rbxts/{name}\", \"main\": \"out/init.lua\", \"types\": \"out/index.d.ts\" }}\n")
}

/// Package files in a subfolder of the package (`types: out/index.d.ts`): written after the watch started, or while
/// the rebuild their package folder started runs (several delays), or with a 1985 modification time (an extracted
/// archive) while it runs. Each is seen: the watcher ends on a successful build.
#[test]
fn package_files_in_subfolders_late_or_old_are_seen() {
    let Some((base, root, links)) = linked_project("nested") else { return };
    let scope = root.join("node_modules/@rbxts");
    // package.json and an empty out/ before the watch, the types file later
    write(&scope.join("neststatic/package.json"), &package_json("neststatic"));
    std::fs::create_dir_all(scope.join("neststatic/out")).unwrap();
    write(
        &root.join("src/shared/nestStatic.ts"),
        "import { v } from \"@rbxts/neststatic\";\n\nexport const w = v.size();\n",
    );
    let watcher = start(&root);
    assert_eq!(watcher.build_end(0)["ok"], false, "the types are missing");
    write(&scope.join("neststatic/out/index.d.ts"), "export declare const v: Array<number>;\n");
    write(&scope.join("neststatic/out/init.lua"), "return { v = {} }\n");
    watcher.until_ok();
    watcher.still_ok("neststatic");
    assert!(root.join("out/shared/nestStatic.luau").is_file());

    // package.json and out/ start a rebuild; the types land while it runs
    for (i, delay) in [0u64, 15, 40, 100].into_iter().enumerate() {
        let name = format!("nestrace{i}");
        write(
            &root.join(format!("src/shared/{name}.ts")),
            &format!("import {{ v }} from \"@rbxts/{name}\";\n\nexport const w = v.size();\n"),
        );
        let end = watcher.settle();
        assert!(end.as_ref().is_some_and(|e| e["ok"] == false), "{name} is missing: {end:?}");
        write(&scope.join(format!("{name}/package.json")), &package_json(&name));
        std::fs::create_dir_all(scope.join(format!("{name}/out"))).unwrap();
        watcher.build_start();
        std::thread::sleep(Duration::from_millis(delay));
        write(&scope.join(format!("{name}/out/index.d.ts")), "export declare const v: Array<number>;\n");
        write(&scope.join(format!("{name}/out/init.lua")), "return { v = {} }\n");
        watcher.settled_ok(&format!("types {delay} ms into the rebuild"));
    }

    // the types re-export a file that lands, with an old modification time, while the rebuild runs
    for (i, delay) in [0u64, 30, 80].into_iter().enumerate() {
        let name = format!("oldm{i}");
        write(
            &root.join(format!("src/shared/{name}.ts")),
            &format!("import {{ extra }} from \"@rbxts/{name}\";\n\nexport const w = extra;\n"),
        );
        let end = watcher.settle();
        assert!(end.as_ref().is_some_and(|e| e["ok"] == false), "{name} is missing: {end:?}");
        write(&scope.join(format!("{name}/package.json")), &package_json(&name));
        write(&scope.join(format!("{name}/out/index.d.ts")), "export * from \"./extra\";\n");
        write(&scope.join(format!("{name}/out/init.lua")), "return {}\n");
        watcher.build_start();
        std::thread::sleep(Duration::from_millis(delay));
        let extra = scope.join(format!("{name}/out/extra.d.ts"));
        write(&extra, "export declare const extra: number;\n");
        set_old(&extra);
        watcher.settled_ok(&format!("an old file {delay} ms into the rebuild"));
    }

    for link in links {
        unlink(&link);
    }
    drop(watcher);
    let _ = std::fs::remove_dir_all(&base);
}

/// node_modules removed (`npm ci`, `rm -rf node_modules`), a source edited while it is gone, then node_modules back:
/// made again folder by folder, or renamed in whole. The watcher ends on a successful build with the edit.
#[test]
fn node_modules_removed_and_reinstalled_is_seen() {
    for mode in ["copy", "rename"] {
        let Some((base, root, links)) = linked_project(&format!("nmwipe-{mode}")) else { return };
        let watcher = start(&root);
        assert_eq!(watcher.build_end(0)["ok"], true);
        for link in &links {
            unlink(link);
        }
        remove_tree(&root.join("node_modules"));
        let end = watcher.settle();
        assert!(end.as_ref().is_some_and(|e| e["ok"] == false), "{mode}: no node_modules: {end:?}");
        let b = std::fs::read_to_string(root.join("src/shared/b.ts")).unwrap();
        std::fs::write(root.join("src/shared/b.ts"), b + "\nexport const whileGone = 1;\n").unwrap();
        let end = watcher.settle();
        assert!(end.as_ref().is_some_and(|e| e["ok"] == false), "{mode}: the edit while it is gone: {end:?}");
        let links = if mode == "copy" {
            link_packages(&root.join("node_modules/@rbxts"))
        } else {
            let staged = base.join("staged-node_modules");
            let links = link_packages(&staged.join("@rbxts"));
            std::fs::rename(&staged, root.join("node_modules")).unwrap();
            links.iter().map(|l| root.join("node_modules").join(l.strip_prefix(&staged).unwrap())).collect()
        };
        watcher.until_ok();
        watcher.still_ok(mode);
        let b = std::fs::read_to_string(root.join("out/shared/b.luau")).unwrap();
        assert!(b.contains("whileGone"), "{mode}: the edit made while node_modules was gone:\n{b}");
        for link in links {
            unlink(&link);
        }
        drop(watcher);
        let _ = std::fs::remove_dir_all(&base);
    }
}

/// A watch started before node_modules exists (a fresh clone), then the install: rebuilt.
#[test]
fn a_watch_started_before_the_install_sees_it() {
    let Some((base, root, links)) = linked_project("nmfirst") else { return };
    for link in &links {
        unlink(link);
    }
    remove_tree(&root.join("node_modules"));
    let watcher = start(&root);
    assert_eq!(watcher.build_end(0)["ok"], false, "no node_modules");
    let links = link_packages(&root.join("node_modules/@rbxts"));
    watcher.until_ok();
    watcher.still_ok("installed");
    assert!(root.join("out/shared/d.luau").is_file());
    for link in links {
        unlink(&link);
    }
    drop(watcher);
    let _ = std::fs::remove_dir_all(&base);
}
