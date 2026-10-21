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
        let _ = Command::new("cmd").args(["/C", "rmdir"]).arg(link.to_string_lossy().replace('/', "\\")).status();
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
