//! A plugin state file in `rbxtsc -w`, with the `stamp` test plugin (its state file `.stamp-state` in the project
//! directory), in a process of its own: this test binary is the compiler when `RBXTS_TEST_CHILD_ARGS` is set (the
//! `child` test). The watcher's own write of the state file and the same bytes written again start nothing; the file
//! deleted, edited or put back to the bytes before the last commit starts a rebuild in which the plugin starts from
//! a fresh state and every file is transformed.

#![cfg(windows)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use serde_json::Value;

const DEFAULT_NODE_MODULES: &str = "E:/Projects/TS/Flamework/roblox-ts-test/roblox-ts/tests/node_modules";
const CHILD_ARGS: &str = "RBXTS_TEST_CHILD_ARGS";
const STATE: &str = ".stamp-state";

/// The compiler with `stamp`, when this process was started as one (its arguments in `RBXTS_TEST_CHILD_ARGS`, one
/// per line); otherwise nothing.
#[test]
fn child() {
    let Ok(args) = std::env::var(CHILD_ARGS) else { return };
    let args: Vec<String> = args.lines().map(str::to_owned).collect();
    let status = rbxts_cli::Cli::new().erased_plugin(rbxts_test_plugins::by_name("stamp").unwrap()).run_with(args);
    std::process::exit(i32::from(status));
}

/// A copy of the basic watch fixture (its tsconfig turns `stamp` on) with a linked node_modules; removed on drop.
struct Project(PathBuf);

impl Project {
    fn new(label: &str) -> Option<Project> {
        let node_modules = std::env::var_os("RBXTS_RS_FIXTURE_NODE_MODULES")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_NODE_MODULES));
        if !node_modules.join("@rbxts").join("compiler-types").is_dir() {
            eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
            return None;
        }
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../rbxts_transform/tests/watch_fixtures/basic");
        let root = std::env::temp_dir().join(format!("rbxts-rs-cli-state-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        copy_dir(&fixture, &root);
        let project = Project(root);
        let status = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(project.0.join("node_modules").to_string_lossy().replace('/', "\\"))
            .arg(node_modules.to_string_lossy().replace('/', "\\"))
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "could not link node_modules");
        Some(project)
    }

    fn write(&self, relative: &str, text: &str) {
        std::fs::write(self.0.join(relative), text).unwrap();
    }

    fn read(&self, relative: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(relative)).ok()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = Command::new("cmd")
            .args(["/C", "rmdir"])
            .arg(self.0.join("node_modules").to_string_lossy().replace('/', "\\"))
            .stderr(Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&self.0);
    }
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

/// The watcher process; killed on drop.
struct Watcher {
    child: Child,
    events: Receiver<Value>,
    lines: Receiver<String>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Watcher {
    fn start(project: &Project) -> Watcher {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child", "--nocapture", "--test-threads=1"])
            .env(CHILD_ARGS, ["-w", "--events", "json"].join("\n"))
            .env_remove("STAMP_PREFIX")
            .env("RBXTS_WATCH_MEMORY_LIMIT_MB", "0")
            .env("RBXTS_WATCH_CHILD", "1")
            .current_dir(&project.0)
            .stdin(Stdio::piped())
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
        Watcher { child, events, lines }
    }

    /// The next `build-end` event within 60 s; it must have succeeded.
    fn next_end(&self) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let Ok(event) = self.events.recv_timeout(left) else {
                let output: Vec<String> = self.lines.try_iter().collect();
                panic!("no build-end within 60 s; output:\n{}", output.join("\n"));
            };
            if event["event"] == "build-end" {
                assert_eq!(event["ok"], true, "{event}");
                return event;
            }
        }
    }

    /// No build starts within `quiet`.
    fn assert_quiet(&self, quiet: Duration) {
        if let Ok(event) = self.events.recv_timeout(quiet) {
            panic!("unexpected event {event}");
        }
    }
}

#[test]
fn a_state_file_deleted_or_edited_while_watching_starts_its_plugin_fresh() {
    let Some(project) = Project::new("watch") else { return };
    let watcher = Watcher::start(&project);
    let quiet = Duration::from_millis(2000);
    let end = watcher.next_end();
    assert_eq!(end["all"], true, "{end}");
    assert_eq!(project.read(STATE).as_deref(), Some("1"));
    // the watcher's own write of the state file (and of the artifact) starts nothing
    watcher.assert_quiet(quiet);

    // an edit: the closure; the state file written again (its own write) starts nothing
    let d = "src/shared/d.ts";
    let text = project.read(d).unwrap();
    project.write(d, &format!("{text}\nexport const sfile = 1;\n"));
    let end = watcher.next_end();
    assert_eq!(end["all"], false, "{end}");
    assert_eq!(project.read(STATE).as_deref(), Some("2"));
    watcher.assert_quiet(quiet);

    // the same bytes written again: nothing
    project.write(STATE, "2");
    watcher.assert_quiet(quiet);

    // deleted: a rebuild, the plugin fresh (its build count starts over), every file
    std::fs::remove_file(project.0.join(STATE)).unwrap();
    let end = watcher.next_end();
    assert_eq!(end["all"], true, "{end}");
    assert_eq!(project.read(STATE).as_deref(), Some("1"));
    assert!(project.read("stamp-report.json").unwrap_or_default().ends_with(",\"builds\":1}\n"));
    watcher.assert_quiet(quiet);

    // edited
    project.write(STATE, "41");
    let end = watcher.next_end();
    assert_eq!(end["all"], true, "{end}");
    assert_eq!(project.read(STATE).as_deref(), Some("1"));
    watcher.assert_quiet(quiet);

    // the bytes before the last commit
    project.write(STATE, "41");
    let end = watcher.next_end();
    assert_eq!(end["all"], true, "{end}");
    assert_eq!(project.read(STATE).as_deref(), Some("1"));
    watcher.assert_quiet(quiet);
    drop(watcher);
}
