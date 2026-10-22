//! Incremental and watch builds (DESIGN §14) on `tests/watch_fixtures/basic`:
//! - the W1 harness (`rbxts_transform::build::parity`) runs `basic.script.json` as watch rebuilds with the `stamp`
//!   plugin (artifacts, a state file, late strings re-rendered in cached files), without plugins, and as
//!   incremental builds in new sessions; after every step the output must equal a clean build's;
//! - `attack.script.json` on `tests/watch_fixtures/attack`: the review of the branch's attack sequence;
//! - the kept-`Shared` contract, with a plugin that counts its `setup` and `rebuild_start` calls;
//! - the clean of a declaration build over a used output directory, against roblox-ts's keep set, with a junction.
//!
//! Like `tests/plugins.rs`, the fixture links a `node_modules` (`RBXTS_RS_FIXTURE_NODE_MODULES`, else the roblox-ts
//! tests project's); without one the tests are skipped.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rbxts_plugin::erased::{DynPlugin, erase};
use rbxts_plugin::{
    BuildKind, ChangeSet, CheckerInfo, Cx, Fatal, FileIndex, FinishCx, Interest, PResult, Plugin, PluginInfo,
    PluginInstance, RebuildPlan, SetupCx, StateOrigin,
};
use rbxts_project::{ProjectOptionLayer, STACK_SIZE};
use rbxts_transform::build::parity::{self, ParityOptions, StepResult};
use rbxts_transform::{BuildSession, CompileOptions, PluginOptions};

const DEFAULT_NODE_MODULES: &str = "E:/Projects/TS/Flamework/roblox-ts-test/roblox-ts/tests/node_modules";

fn node_modules() -> Option<PathBuf> {
    let path = std::env::var_os("RBXTS_RS_FIXTURE_NODE_MODULES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_NODE_MODULES));
    path.join("@rbxts").join("compiler-types").is_dir().then_some(path)
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/watch_fixtures")
}

/// Runs `f` on a thread with the compiler's stack.
fn big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new().stack_size(STACK_SIZE).spawn_scoped(scope, f).unwrap().join().unwrap()
    })
}

fn run_basic(label: &str, plugins: &[&str], incremental: bool) -> Option<Vec<StepResult>> {
    run_fixture("basic", label, plugins, incremental)
}

fn run_fixture(fixture: &str, label: &str, plugins: &[&str], incremental: bool) -> Option<Vec<StepResult>> {
    let Some(node_modules) = node_modules() else {
        eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
        return None;
    };
    let script =
        parity::parse_script(&std::fs::read_to_string(fixtures().join(format!("{fixture}.script.json"))).unwrap())
            .unwrap();
    let scratch = std::env::temp_dir().join(format!("rbxts-rs-watch-{label}-{}", std::process::id()));
    let plugins: Vec<Box<dyn DynPlugin>> =
        plugins.iter().map(|name| rbxts_test_plugins::by_name(name).expect("a test plugin")).collect();
    let results = big_stack(|| {
        let options = ParityOptions {
            source: fixtures().join(fixture),
            tsconfig: "tsconfig.json".into(),
            copy: ["src", "tsconfig.json", "default.project.json", "package.json"].map(str::to_owned).to_vec(),
            scratch: scratch.clone(),
            project_options: ProjectOptionLayer::default(),
            plugins: &plugins,
            ignore_unknown_plugins: plugins.is_empty(),
            node_modules: Some(node_modules),
            node_modules_mode: parity::NodeModules::LinkPackages,
            incremental,
            keep_clean_copies: false,
            check_clean: true,
            memory: None,
        };
        parity::run(&options, &script)
    })
    .unwrap();
    parity::remove_tree(&scratch);
    for (index, result) in results.iter().enumerate() {
        assert!(result.problems.is_empty(), "step {index} `{}`: {:#?}", result.name, result.problems);
    }
    assert_eq!(results.len(), script.steps.len() + 1, "every step ran");
    Some(results)
}

fn step<'r>(results: &'r [StepResult], name: &str) -> &'r StepResult {
    results.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no step {name}"))
}

#[test]
fn watch_rebuilds_equal_clean_builds_with_the_stamp_plugin() {
    let Some(results) = run_basic("stamp", &["stamp"], false) else { return };
    // the closure, not every file
    let chain = step(&results, "C → B → D: Inner.a array to map");
    assert!(!chain.all && chain.transformed > 0);
    assert!(matches!(chain.kind, Some(BuildKind::WatchRebuild { .. })));
    // the global rule: every file
    assert!(step(&results, "S1: a global .d.ts interface").all);
    assert!(step(&results, "S1b: declare global in a module").all);
    // the augmentation edge reaches useAug.ts, which does not import aug.ts (checked by the parity itself)
    assert!(!step(&results, "S2: a module augmentation").all);
    // a late-string renumbering touches one source file
    assert_eq!(step(&results, "a new late string renumbers cached files").transformed, 1);
    // a tsconfig change re-runs setup: every file
    assert!(step(&results, "a tsconfig edit (the plugin's options)").all);
    assert_eq!(step(&results, "an unchanged save").transformed, 0);
}

/// The review's attack sequence (globals reached through the closure, augmentation and export hops, const enums,
/// JSDoc macro modes, renames, deletions and re-adds, a twin .d.ts, tsconfig and Rojo edits, a case-only rename).
#[test]
fn the_review_attack_sequence_equals_clean_builds() {
    run_fixture("attack", "attack", &["stamp"], false);
    run_fixture("attack", "attack-inc", &[], true);
}

#[test]
fn watch_rebuilds_equal_clean_builds_without_plugins() {
    run_basic("plain", &[], false);
}

#[test]
fn incremental_builds_equal_clean_builds() {
    let Some(results) = run_basic("incremental", &[], true) else { return };
    assert_eq!(results[0].kind, Some(BuildKind::Clean));
    let chain = step(&results, "C → B → D: Inner.a array to map");
    assert_eq!(chain.kind, Some(BuildKind::Incremental));
    assert!(!chain.all && chain.transformed > 0);
}

// --- the kept-Shared contract ---

#[derive(Default)]
struct Counts {
    setups: AtomicUsize,
    rebuild_starts: AtomicUsize,
    memory_origins: AtomicUsize,
}

/// Counts its `setup` and `rebuild_start` calls. Reads `counter-input.txt` (a build input); asks for a new `Shared`
/// when a changed file's name contains `rerun`; its state counts finished builds, written to `counter.txt`.
struct Counter(Arc<Counts>);

struct CounterInstance;

impl Plugin for Counter {
    type Shared = String;
    type State = u32;
    type FilePart = ();
    type Instance = CounterInstance;

    fn info(&self) -> PluginInfo {
        PluginInfo { name: "counter", version: "0.1.0", tsconfig_transform: None }
    }

    fn setup(&self, cx: &mut SetupCx<'_>, _state: &mut u32) -> Result<Arc<String>, Fatal> {
        self.0.setups.fetch_add(1, Ordering::SeqCst);
        if cx.state_origin() == StateOrigin::Memory {
            self.0.memory_origins.fetch_add(1, Ordering::SeqCst);
        }
        let input = cx.read_input("counter-input.txt").map_err(|e| Fatal::new(e.to_string()))?;
        Ok(Arc::new(input.unwrap_or_default()))
    }

    fn rebuild_start(&self, _shared: &String, _state: &u32, changes: &ChangeSet) -> RebuildPlan {
        self.0.rebuild_starts.fetch_add(1, Ordering::SeqCst);
        if changes.changed.iter().chain(&changes.added).any(|f| f.contains("rerun")) {
            RebuildPlan::RerunSetup
        } else {
            RebuildPlan::KeepShared
        }
    }

    fn instance(&self, _shared: Arc<String>, _checker: CheckerInfo) -> CounterInstance {
        CounterInstance
    }

    fn finish(
        &self,
        shared: &String,
        state: &mut u32,
        _parts: &[(FileIndex, &())],
        cx: &mut FinishCx<'_>,
    ) -> Result<(), Fatal> {
        *state += 1;
        cx.write_artifact("counter.txt", format!("{state} {}", shared.trim()).into_bytes());
        Ok(())
    }
}

impl PluginInstance for CounterInstance {
    type FilePart = ();

    fn interest(&self, _interest: &mut Interest) {}

    fn end_file(&self, _cx: &mut Cx<'_>) -> PResult<()> {
        Ok(())
    }
}

/// A copy of the basic fixture with a linked node_modules; removed on drop.
struct Copy(PathBuf);

impl Drop for Copy {
    fn drop(&mut self) {
        parity::remove_tree(&self.0);
    }
}

#[test]
fn a_kept_shared_survives_failures_and_input_changes_rerun_setup() {
    let Some(node_modules) = node_modules() else {
        eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
        return;
    };
    let root = std::env::temp_dir().join(format!("rbxts-rs-watch-shared-{}", std::process::id()));
    parity::remove_tree(&root);
    let copy = Copy(root.clone());
    parity::copy_tree(&fixtures().join("basic"), &root).unwrap();
    parity::junction(&root.join("node_modules"), &node_modules).unwrap();
    std::fs::write(root.join("counter-input.txt"), "one").unwrap();
    let counts = Arc::new(Counts::default());
    let plugins = vec![erase(Counter(Arc::clone(&counts)))];
    let path = |p: &str| root.join(p).to_string_lossy().replace('\\', "/");
    let write = |p: &str, text: &str| std::fs::write(root.join(p), text).unwrap();
    let artifact = || std::fs::read_to_string(root.join("counter.txt")).unwrap_or_default();
    let setups = || counts.setups.load(Ordering::SeqCst);
    let starts = || counts.rebuild_starts.load(Ordering::SeqCst);
    big_stack(|| {
        let options =
            PluginOptions { plugins: &plugins, ignore_unknown_tsconfig_plugins: true, ..PluginOptions::default() };
        let mut session = BuildSession::new(
            CompileOptions {
                tsconfig: "tsconfig.json".into(),
                cwd: root.to_string_lossy().into_owned(),
                project_options: ProjectOptionLayer::default(),
                out_dir_override: None,
                write_partial: false,
                no_write: false,
            },
            &options,
            true,
        );
        let (report, summary) = session.build(&[]);
        assert!(!report.has_errors(), "{:?}", report.fatal);
        assert_eq!(summary.kind, Some(BuildKind::Clean));
        assert_eq!((setups(), starts()), (1, 0));
        assert_eq!(artifact(), "1 one");

        // an edit: rebuild_start keeps the Shared, only the closure is transformed
        write(
            "src/shared/d.ts",
            "import { A } from \"./b\";\n\nexport function sizeOf(value: A) {\n\treturn value.x.a.size() + 1;\n}\n",
        );
        let (report, summary) = session.build(&[path("src/shared/d.ts")]);
        assert!(!report.has_errors());
        assert_eq!((setups(), starts()), (1, 1));
        assert!(!summary.all);
        assert_eq!(artifact(), "2 one", "the committed state is handed on");

        // a build input changed: setup runs again (no rebuild_start), on the committed state, and every file is
        // transformed
        write("counter-input.txt", "two");
        let (report, summary) = session.build(&[path("counter-input.txt")]);
        assert!(!report.has_errors());
        assert_eq!((setups(), starts()), (2, 1));
        assert!(summary.all);
        assert_eq!(counts.memory_origins.load(Ordering::SeqCst), 1);
        assert_eq!(artifact(), "3 two");

        // a failing build keeps the Shared it was handed and writes nothing
        write(
            "src/shared/d.ts",
            "import { A } from \"./b\";\n\nexport function sizeOf(value: A) {\n\treturn value.x.a.sizee();\n}\n",
        );
        let (report, _) = session.build(&[path("src/shared/d.ts")]);
        assert!(report.has_errors());
        assert_eq!((setups(), starts()), (2, 2));
        assert_eq!(artifact(), "3 two");

        // the fix: the kept Shared is still there
        write(
            "src/shared/d.ts",
            "import { A } from \"./b\";\n\nexport function sizeOf(value: A) {\n\treturn value.x.a.size() + 2;\n}\n",
        );
        let (report, _) = session.build(&[path("src/shared/d.ts")]);
        assert!(!report.has_errors());
        assert_eq!((setups(), starts()), (2, 3));
        assert_eq!(artifact(), "4 two");

        // RerunSetup re-transforms everything
        write("src/shared/rerun.ts", "export const rerun = 1;\n");
        let (report, summary) = session.build(&[path("src/shared/rerun.ts")]);
        assert!(!report.has_errors());
        assert_eq!((setups(), starts()), (3, 4));
        assert!(summary.all);
        assert_eq!(artifact(), "5 two");

        // a failing build that made a new Shared drops it: the next build runs setup again
        write("counter-input.txt", "three");
        write(
            "src/shared/d.ts",
            "import { A } from \"./b\";\n\nexport function sizeOf(value: A) {\n\treturn value.x.a.sizee();\n}\n",
        );
        let (report, _) = session.build(&[path("counter-input.txt"), path("src/shared/d.ts")]);
        assert!(report.has_errors());
        assert_eq!((setups(), starts()), (4, 4));
        assert_eq!(artifact(), "5 two");
        write(
            "src/shared/d.ts",
            "import { A } from \"./b\";\n\nexport function sizeOf(value: A) {\n\treturn value.x.a.size();\n}\n",
        );
        let (report, summary) = session.build(&[path("src/shared/d.ts")]);
        assert!(!report.has_errors());
        assert_eq!((setups(), starts()), (5, 4), "the input still differs from the committed one: setup again");
        assert!(summary.all);
        assert_eq!(artifact(), "6 three");
    });
    drop(copy);
}

// --- the clean keeps what roblox-ts keeps ---

/// A declaration build (`declaration`, `declarationMap`, `declarationDir`, an explicit `tsBuildInfoFile` under
/// `outDir`) over a used output directory: the clean keeps every file roblox-ts's `getProjectOutputs` keeps (the
/// `.d.ts` and `.d.ts.map` TypeScript emits for each file, `index.d.ts` beside `init.luau` included; the outputs of
/// project `.d.ts` assets; tsc's build info and roblox-ts's), deletes what it does not, and removes a junction inside
/// the output directory as a link, leaving its target alone.
#[test]
fn the_clean_keeps_what_roblox_ts_keeps_and_removes_links_as_links() {
    let Some(node_modules) = node_modules() else {
        eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
        return;
    };
    let root = std::env::temp_dir().join(format!("rbxts-rs-watch-keep-{}", std::process::id()));
    let sentinel = std::env::temp_dir().join(format!("rbxts-rs-watch-keep-sentinel-{}", std::process::id()));
    parity::remove_tree(&root);
    let copy = Copy(root.clone());
    parity::copy_tree(&fixtures().join("basic"), &root).unwrap();
    parity::junction(&root.join("node_modules"), &node_modules).unwrap();
    let tsconfig = std::fs::read_to_string(root.join("tsconfig.json")).unwrap().replace(
        "\"compilerOptions\": {",
        "\"compilerOptions\": {\n\t\t\"declaration\": true,\n\t\t\"declarationMap\": true,\n\t\t\"declarationDir\": \"types\",\n\t\t\"incremental\": true,\n\t\t\"tsBuildInfoFile\": \"out/info.tsbuildinfo\",",
    );
    std::fs::write(root.join("tsconfig.json"), tsconfig).unwrap();
    std::fs::write(root.join("src/shared/index.ts"), "export const idx = 1;\n").unwrap();
    let put = |p: &str| {
        let path = root.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "user file\n").unwrap();
    };
    let kept = [
        "types/shared/index.d.ts",
        "types/shared/index.d.ts.map",
        "types/shared/c.d.ts",
        "types/shared/c.d.ts.map",
        "out/shared/globals.d.ts",
        "out/info.tsbuildinfo",
        "out/info.rbxtsc.tsbuildinfo",
    ];
    let stale = ["types/shared/gone.d.ts", "out/shared/gone.luau", "out/shared/init.d.ts"];
    for p in kept.iter().chain(&stale) {
        put(p);
    }
    std::fs::create_dir_all(&sentinel).unwrap();
    std::fs::write(sentinel.join("precious.txt"), "keep me\n").unwrap();
    parity::junction(&root.join("out/linked"), &sentinel).unwrap();

    let plugins: Vec<Box<dyn DynPlugin>> = Vec::new();
    let (report, _) = big_stack(|| {
        let options =
            PluginOptions { plugins: &plugins, ignore_unknown_tsconfig_plugins: true, ..PluginOptions::default() };
        let mut session = BuildSession::new(
            CompileOptions {
                tsconfig: "tsconfig.json".into(),
                cwd: root.to_string_lossy().into_owned(),
                project_options: ProjectOptionLayer::default(),
                out_dir_override: None,
                write_partial: false,
                no_write: false,
            },
            &options,
            false,
        );
        let (report, summary) = session.build(&[]);
        (report.fatal.clone(), summary)
    });
    assert!(report.is_empty(), "{report:?}");
    for p in kept {
        assert!(root.join(p).is_file(), "{p} was deleted");
    }
    for p in stale {
        assert!(!root.join(p).exists(), "{p} was kept");
    }
    assert!(root.join("out/shared/init.luau").is_file());
    assert!(std::fs::symlink_metadata(root.join("out/linked")).is_err(), "the junction is removed");
    assert_eq!(std::fs::read_to_string(sentinel.join("precious.txt")).unwrap(), "keep me\n", "its target is not");
    let _ = std::fs::remove_dir_all(&sentinel);
    drop(copy);
}

/// Writes one artifact at a fixed path.
struct ArtifactAt(&'static str);

impl Plugin for ArtifactAt {
    type Shared = ();
    type State = ();
    type FilePart = ();
    type Instance = CounterInstance;

    fn info(&self) -> PluginInfo {
        PluginInfo { name: "artifact-at", version: "0.1.0", tsconfig_transform: None }
    }

    fn setup(&self, _cx: &mut SetupCx<'_>, _state: &mut ()) -> Result<Arc<()>, Fatal> {
        Ok(Arc::new(()))
    }

    fn instance(&self, _shared: Arc<()>, _checker: CheckerInfo) -> CounterInstance {
        CounterInstance
    }

    fn finish(
        &self,
        _shared: &(),
        _state: &mut (),
        _parts: &[(FileIndex, &())],
        cx: &mut FinishCx<'_>,
    ) -> Result<(), Fatal> {
        cx.write_artifact(self.0, b"plugin bytes\n".to_vec());
        Ok(())
    }
}

/// A plugin artifact at an asset's output path fails the build, naming the plugin, and the asset is not overwritten.
#[test]
fn an_artifact_may_not_overwrite_an_asset_output() {
    let Some(node_modules) = node_modules() else {
        eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
        return;
    };
    let root = std::env::temp_dir().join(format!("rbxts-rs-watch-artifact-{}", std::process::id()));
    parity::remove_tree(&root);
    let copy = Copy(root.clone());
    parity::copy_tree(&fixtures().join("basic"), &root).unwrap();
    parity::junction(&root.join("node_modules"), &node_modules).unwrap();
    let plugins = vec![erase(ArtifactAt("out/shared/asset.lua"))];
    let fatal = big_stack(|| {
        let options =
            PluginOptions { plugins: &plugins, ignore_unknown_tsconfig_plugins: true, ..PluginOptions::default() };
        let mut session = BuildSession::new(
            CompileOptions {
                tsconfig: "tsconfig.json".into(),
                cwd: root.to_string_lossy().into_owned(),
                project_options: ProjectOptionLayer::default(),
                out_dir_override: None,
                write_partial: false,
                no_write: false,
            },
            &options,
            false,
        );
        session.build(&[]).0.fatal
    });
    assert!(fatal.iter().any(|f| f.contains("artifact-at") && f.contains("is an output of this build")), "{fatal:?}");
    assert!(!root.join("out").exists(), "a failing build writes nothing");
    drop(copy);
}

#[test]
fn a_watch_rebuild_load_checks_what_a_plugin_emitted_into() {
    let Some(node_modules) = node_modules() else {
        eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
        return;
    };
    let root = std::env::temp_dir().join(format!("rbxts-rs-watch-luau-{}", std::process::id()));
    parity::remove_tree(&root);
    let copy = Copy(root.clone());
    parity::copy_tree(&fixtures().join("basic"), &root).unwrap();
    parity::junction(&root.join("node_modules"), &node_modules).unwrap();
    let plugins = vec![rbxts_test_plugins::by_name("identity-call").expect("the identity-call plugin")];
    let path = |p: &str| root.join(p).to_string_lossy().replace('\\', "/");
    // `count` locals from calls the plugin claims: Luau allows 200
    let big = |count: usize| {
        let locals: String = (0..count).map(|i| format!("const a{i} = mk();\n")).collect();
        std::fs::write(
            root.join("src/shared/big.ts"),
            format!("declare function mk(): number;\n{locals}export const total = a0 + a1;\n"),
        )
        .unwrap();
    };
    let output = root.join("out/shared/big.luau");
    big_stack(|| {
        let options =
            PluginOptions { plugins: &plugins, ignore_unknown_tsconfig_plugins: true, ..PluginOptions::default() };
        let mut session = BuildSession::new(
            CompileOptions {
                tsconfig: "tsconfig.json".into(),
                cwd: root.to_string_lossy().into_owned(),
                project_options: ProjectOptionLayer::default(),
                out_dir_override: None,
                write_partial: false,
                no_write: false,
            },
            &options,
            true,
        );
        big(150);
        let (report, _) = session.build(&[]);
        assert!(!report.has_errors(), "{:?}", report.fatal);
        assert!(report.timings.load_checked > 0);
        let written = std::fs::read_to_string(&output).unwrap();

        // the rebuild Luau refuses fails, and the old output stays as it was
        big(205);
        let (report, _) = session.build(&[path("src/shared/big.ts")]);
        let refused: Vec<String> = report
            .files
            .iter()
            .flat_map(|f| f.diagnostics.iter())
            .map(|d| tsrs_compiler::diagnosticwriter::flatten_diagnostic_message(*d, " "))
            .collect();
        assert!(
            refused.iter().any(|m| m.contains("refuses this file") && m.contains("Out of local registers")),
            "{refused:?}"
        );
        assert_eq!(std::fs::read_to_string(&output).unwrap(), written, "a failing rebuild writes nothing");

        // fixed, it is written
        big(190);
        let (report, _) = session.build(&[path("src/shared/big.ts")]);
        assert!(!report.has_errors(), "{:?}", report.fatal);
        assert!(std::fs::read_to_string(&output).unwrap().contains("local a189 = mk()"));
    });
    drop(copy);
}
