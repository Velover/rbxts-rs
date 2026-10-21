//! `Cli` end to end on a copy of `rbxts_transform`'s probe fixture: registered plugins build it (exit 0), and the
//! plain command line, without the plugin the tsconfig names, refuses it (exit 1), as roblox-ts's exit codes go.

#![cfg(windows)]

use std::path::{Path, PathBuf};

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

#[test]
fn builds_with_registered_plugins_and_returns_roblox_ts_exit_codes() {
    let node_modules = std::env::var_os("RBXTS_RS_FIXTURE_NODE_MODULES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_NODE_MODULES));
    if !node_modules.join("@rbxts").join("compiler-types").is_dir() {
        eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
        return;
    }
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../rbxts_transform/tests/plugin_fixtures/probe");
    let root = std::env::temp_dir().join(format!("rbxts-rs-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for name in ["src", "tsconfig.json", "default.project.json", "package.json"] {
        let from = fixture.join(name);
        if from.is_dir() {
            copy_dir(&from, &root.join(name));
        } else {
            std::fs::create_dir_all(&root).unwrap();
            std::fs::copy(&from, root.join(name)).unwrap();
        }
    }
    let link = root.join("node_modules");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(node_modules.to_string_lossy().replace('/', "\\"))
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "could not link node_modules");
    let project = root.to_string_lossy().into_owned();

    // the tsconfig lists rbxts-transform-stamp, which the plain command line does not have
    let plain = rbxts_cli::Cli::new().run_with(vec!["build".into(), "-p".into(), project.clone()]);
    assert_eq!(plain, 1);
    assert!(!root.join("out").exists());

    let status = rbxts_cli::Cli::new()
        .erased_plugin(rbxts_test_plugins::by_name("stamp").unwrap())
        .erased_plugin(rbxts_test_plugins::by_name("probe").unwrap())
        .run_with(vec!["-p".into(), project.clone(), "--timings".into()]);
    assert_eq!(status, 0);
    assert!(root.join("out/shared/calls.luau").is_file());
    assert!(root.join("stamp-report.json").is_file());

    // a hook error fails the build: exit 1
    let status = rbxts_cli::Cli::new()
        .erased_plugin(rbxts_test_plugins::by_name("stamp").unwrap())
        .erased_plugin(rbxts_test_plugins::by_name("probe-fail").unwrap())
        .run_with(vec!["-p".into(), project]);
    assert_eq!(status, 1);

    let _ = std::process::Command::new("cmd").args(["/C", "rmdir"]).arg(&link).status();
    let _ = std::fs::remove_dir_all(&root);
}
