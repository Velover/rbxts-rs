//! The compiler side of the plugin seam against the real transform: the `stamp` and `probe` test plugins
//! (`rbxts_test_plugins`) over `tests/plugin_fixtures/probe`, whose `expected/` Luau is hand-checked (plugin output
//! has no stock roblox-ts reference).
//!
//! Covered: calls by symbol, by receiver (plain and inside an optional chain) and by callee marker; `new` and
//! decorator claims; `compile_call` with `Original` / `Luau` / `Nil` slots and `Function` / `Method` callees;
//! `compile_in_order`, prereqs, comments, captures, hoists, root insertions and the header; `import_named` (new and
//! reused imports) and `request_import_elision` / `mark_import_used`; the class hook; the expression and statement
//! fallbacks; late strings; source facts (UTF-16 positions, `{@link}`, `declaring_package`, `resolve_module`); the
//! artifact and state-file commit; claim order and `RBXTS_PLUGIN_CLAIMS=all`; the phase-guard refusals; a
//! panicking hook; `Err` without an error; an unresolved late string; an artifact path outside the project;
//! `explain_checker_diagnostic`; and tsconfig activation.
//!
//! Like `tests/fixtures.rs`, the fixture is copied to a temporary directory with a `node_modules` junction
//! (`RBXTS_RS_FIXTURE_NODE_MODULES`, else the roblox-ts tests project's); without one the tests are skipped.

#![cfg(windows)]

use std::path::{Path, PathBuf};

use rbxts_plugin::erased::DynPlugin;
use rbxts_project::{ProjectOptionLayer, STACK_SIZE};
use rbxts_transform::{CompileOptions, CompileReport, PluginOptions, compile_project_with};
use tsrs_compiler::diagnosticwriter::flatten_diagnostic_message;
use tsrs_diagnostics::Category;

const DEFAULT_NODE_MODULES: &str = "E:/Projects/TS/Flamework/roblox-ts-test/roblox-ts/tests/node_modules";

fn node_modules() -> Option<PathBuf> {
    let path = std::env::var_os("RBXTS_RS_FIXTURE_NODE_MODULES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_NODE_MODULES));
    path.join("@rbxts").join("compiler-types").is_dir().then_some(path)
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/plugin_fixtures/probe")
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

/// A copy of the fixture in a temporary directory, removed on drop (the junction with `rmdir`, never followed).
struct Project {
    root: PathBuf,
}

impl Project {
    /// `None` (the test is skipped) without a `node_modules` to link.
    fn new(label: &str) -> Option<Project> {
        let Some(node_modules) = node_modules() else {
            eprintln!("skipped: no node_modules with @rbxts/compiler-types (set RBXTS_RS_FIXTURE_NODE_MODULES)");
            return None;
        };
        let root = std::env::temp_dir().join(format!("rbxts-rs-plugins-{label}-{}", std::process::id()));
        let project = Project { root };
        project.clean();
        let fixture = fixture_dir();
        for name in ["src", "tsconfig.json", "default.project.json", "package.json"] {
            let from = fixture.join(name);
            if from.is_dir() {
                copy_dir(&from, &project.root.join(name));
            } else {
                std::fs::copy(&from, project.root.join(name)).unwrap();
            }
        }
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(project.root.join("node_modules"))
            .arg(node_modules.to_string_lossy().replace('/', "\\"))
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "could not link node_modules");
        Some(project)
    }

    fn clean(&self) {
        let link = self.root.join("node_modules");
        if link.exists() {
            let _ = std::process::Command::new("cmd").args(["/C", "rmdir"]).arg(&link).status();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }

    fn write(&self, relative: &str, text: &str) {
        std::fs::write(self.root.join(relative), text).unwrap();
    }

    /// Builds the copy with the named test plugins (in registration order).
    fn build(&self, plugins: &[&str], configure: impl FnOnce(&mut PluginOptions<'_>)) -> CompileReport {
        let plugins: Vec<Box<dyn DynPlugin>> =
            plugins.iter().map(|name| rbxts_test_plugins::by_name(name).expect("a test plugin")).collect();
        let cwd = self.root.to_string_lossy().into_owned();
        let mut options = PluginOptions { plugins: &plugins, ..PluginOptions::default() };
        configure(&mut options);
        let ignore = options.ignore_unknown_tsconfig_plugins;
        let claims_all = options.claims_all;
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(STACK_SIZE)
                .spawn_scoped(scope, move || {
                    let options =
                        PluginOptions { plugins: &plugins, ignore_unknown_tsconfig_plugins: ignore, claims_all };
                    compile_project_with(
                        &CompileOptions {
                            tsconfig: "tsconfig.json".into(),
                            cwd,
                            project_options: ProjectOptionLayer::default(),
                            out_dir_override: None,
                            write_partial: false,
                            no_write: false,
                        },
                        &options,
                    )
                })
                .unwrap()
                .join()
                .unwrap()
        })
    }

    fn out_exists(&self) -> bool {
        self.root.join("out").exists()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        self.clean();
    }
}

/// Every diagnostic of the build, as `error|warning <file>: <message>`.
fn diagnostics(report: &CompileReport) -> Vec<String> {
    let mut all = report.diagnostics.clone();
    for file in &report.files {
        all.extend(file.diagnostics.iter().copied());
    }
    all.iter()
        .map(|d| {
            let category = if d.category() == Category::Error { "error" } else { "warning" };
            let file = d.file().map(|f| f.file_name().rsplit('/').next().unwrap_or_default().to_owned());
            format!("{category} {}: {}", file.unwrap_or_default(), flatten_diagnostic_message(*d, "\n"))
        })
        .collect()
}

fn errors(report: &CompileReport) -> Vec<String> {
    diagnostics(report).into_iter().filter(|d| d.starts_with("error")).collect()
}

/// Compares the build's Luau with `expected/`.
fn assert_expected(report: &CompileReport) {
    assert_expected_except(report, &[]);
}

/// Compares the build's Luau with `expected/`, but for the files in `except`.
fn assert_expected_except(report: &CompileReport, except: &[&str]) {
    let mut failures = Vec::new();
    let expected_dir = fixture_dir().join("expected");
    for file in &report.files {
        let relative = Path::new(&file.out_path)
            .strip_prefix(&report.out_dir)
            .unwrap_or(Path::new(&file.out_path))
            .to_string_lossy()
            .replace('\\', "/");
        if except.contains(&relative.as_str()) {
            continue;
        }
        let Some(luau) = &file.luau else {
            failures.push(format!("{relative}: no output"));
            continue;
        };
        let expected = std::fs::read_to_string(expected_dir.join(&relative))
            .unwrap_or_else(|e| panic!("{relative}: {e}"))
            .replace("\r\n", "\n");
        if *luau != expected {
            failures.push(format!("{relative}: differs from expected\n--- ours\n{luau}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn probe_and_stamp_match_the_expected_luau() {
    let Some(project) = Project::new("expected") else { return };
    // probe-exprcall: an expression-kind hook on `probeExpr…` calls, which the call hooks still claim inside
    let report = project.build(&["stamp", "probe", "probe-exprcall"], |_| {});
    assert!(report.fatal.is_empty(), "{:?}", report.fatal);
    assert_eq!(errors(&report), Vec::<String>::new());
    assert_eq!(report.plugins, vec!["stamp", "probe", "probe-exprcall"]);
    // the unknown @stamp mode is a warning from the hook, per call
    assert_eq!(
        diagnostics(&report),
        vec!["warning calls.ts: `mystery` has an unknown @stamp mode; the call is left as it is"]
    );
    assert_expected(&report);
    // every output was written, then the artifacts and the state file
    assert!(report.files.iter().all(|f| f.written));
    let read = |name: &str| std::fs::read_to_string(project.root.join(name)).unwrap();
    assert_eq!(read("stamp-report.json"), "{\"sites\":1,\"builds\":1}\n");
    assert_eq!(read(".stamp-state"), "1");
    assert_eq!(
        read("probe-report.json"),
        "[{\"file\":\"src/shared/approved.ts\",\"claims\":4},{\"file\":\"src/shared/approvedAfter.ts\",\"claims\":2},\
         {\"file\":\"src/shared/calls.ts\",\"claims\":10},{\"file\":\"src/shared/capture.ts\",\"claims\":2},\
         {\"file\":\"src/shared/classes.ts\",\"claims\":2},{\"file\":\"src/shared/decorators.ts\",\"claims\":2},\
         {\"file\":\"src/shared/elide.ts\",\"claims\":3},{\"file\":\"src/shared/elideAll.ts\",\"claims\":1},\
         {\"file\":\"src/shared/elideKept.ts\",\"claims\":1},{\"file\":\"src/shared/exprcall.ts\",\"claims\":3},{\"file\":\"src/shared/facts.ts\",\"claims\":1},\
         {\"file\":\"src/shared/fallbacks.ts\",\"claims\":2},{\"file\":\"src/shared/inorder.ts\",\"claims\":5},{\"file\":\"src/shared/receivers.ts\",\"claims\":3},\
         {\"file\":\"src/shared/reuse.ts\",\"claims\":1},{\"file\":\"src/shared/selfcalls.ts\",\"claims\":5},\
         {\"file\":\"src/shared/shadow.ts\",\"claims\":3}]\n"
    );
    assert_eq!(read("probe-exprcall-report.json"), "[{\"file\":\"src/shared/exprcall.ts\",\"claims\":3}]\n");
    assert_eq!(report.artifacts_written.len(), 4, "{:?}", report.artifacts_written);
    // a second build changes no Luau and no artifact (write if changed); the stamp state counts the clean build anew
    let again = project.build(&["stamp", "probe", "probe-exprcall"], |_| {});
    assert!(again.files.iter().all(|f| !f.written));
    assert!(again.artifacts_written.is_empty(), "{:?}", again.artifacts_written);
    // setup's env read is a build input
    assert!(
        report
            .build_inputs
            .iter()
            .any(|i| matches!(i, rbxts_transform::plugins::BuildInput::Env { name, .. } if name == "STAMP_PREFIX"))
    );
    // hook timings were recorded per plugin
    let probe = report.plugin_timings.iter().find(|t| t.name == "probe").unwrap();
    let calls = probe.hooks.iter().find(|(k, _)| k.label() == "call").unwrap().1.calls;
    assert!(calls >= 23, "{calls}");
}

#[test]
fn the_first_plugin_to_take_a_call_wins_and_double_claims_are_reported_on_request() {
    let Some(project) = Project::new("order") else { return };
    let report = project.build(&["stamp", "probe", "probe-b"], |_| {});
    assert_eq!(errors(&report), Vec::<String>::new());
    // `probe` takes every node it is asked about first; `probe-b` gets only what `probe` declined (nothing here)
    assert_expected_except(&report, &["shared/classes.luau", "shared/decorators.luau", "shared/exprcall.luau"]);
    let read = |name: &str| std::fs::read_to_string(project.root.join(name)).unwrap();
    assert_eq!(
        read("probe-b-report.json"),
        "[]
"
    );
    // the class hook is broadcast: both plugins' statements, in registration order, before the decorators
    let classes = report.files.iter().find(|f| f.file_name.ends_with("classes.ts")).unwrap().luau.as_deref().unwrap();
    assert!(
        classes.contains(
            "	Decorated.__probed = \"probe\"
	Decorated.__probed = \"probe-b\"
	Decorated = probeDecorator(\"probed\")"
        ),
        "{classes}"
    );

    let report = project.build(&["stamp", "probe", "probe-b"], |o| o.claims_all = true);
    let doubles: Vec<String> = errors(&report).into_iter().filter(|e| e.contains("both claimed")).collect();
    assert!(!doubles.is_empty(), "{:?}", errors(&report));
    assert!(
        doubles
            .iter()
            .all(|e| e.ends_with("plugins `probe` and `probe-b` both claimed this call (RBXTS_PLUGIN_CLAIMS=all)"))
    );
}

#[test]
fn hook_failures_fail_the_build_and_write_nothing() {
    let Some(project) = Project::new("failures") else { return };
    let cases: &[(&str, &[&str])] = &[
        ("probe-fail", &["error failures.ts: probe: fail was called"]),
        (
            "probe-err",
            &[
                "error failures.ts: probe: the first err",
                "error failures.ts: rbxts-rs internal error: plugin `probe` returned Err from call without reporting an error",
            ],
        ),
        ("probe-panic", &["error failures.ts: rbxts-rs internal error: plugin `probe` panicked in call: probe: boom"]),
    ];
    for (plugin, expected) in cases {
        let report = project.build(&["stamp", plugin], |_| {});
        assert_eq!(errors(&report), *expected, "{plugin}");
        assert!(!project.out_exists(), "{plugin}: nothing is written");
        assert!(!project.root.join("probe-report.json").exists(), "{plugin}: no artifact");
        if *plugin == "probe-panic" {
            // the checker may be inconsistent after a panic: no later file is transformed
            assert_eq!(report.files.last().unwrap().file_name.rsplit('/').next(), Some("failures.ts"));
            assert!(
                report.fatal[0].starts_with("the transform stopped after an internal error in"),
                "{:?}",
                report.fatal
            );
        }
    }
}

#[test]
fn refused_primitives_are_internal_errors_naming_the_plugin() {
    let Some(project) = Project::new("refuse") else { return };
    let report = project.build(&["stamp", "probe-refuse"], |_| {});
    let errors = errors(&report);
    for expected in [
        "error calls.ts: rbxts-rs internal error: plugin `probe` called prereq in begin_file",
        "error calls.ts: rbxts-rs internal error: plugin `probe` called late_string in classify_symbol",
        "error calls.ts: rbxts-rs internal error: plugin `probe` called hoist in end_file",
    ] {
        assert!(errors.iter().any(|e| e == expected), "{expected}\nin {errors:?}");
    }
    assert!(!project.out_exists());
}

#[test]
fn finish_errors_fail_the_build() {
    let Some(project) = Project::new("finish") else { return };
    let report = project.build(&["stamp", "probe-unresolved"], |_| {});
    assert_eq!(
        report.fatal,
        vec!["plugin `probe` left the late string `src/shared/elide.ts:0` unresolved (used in src/shared/elide.ts)"]
    );
    assert!(!project.out_exists());

    let report = project.build(&["stamp", "probe-artifact-outside"], |_| {});
    assert_eq!(report.fatal.len(), 1, "{:?}", report.fatal);
    assert!(report.fatal[0].starts_with("plugin `probe`: artifact "), "{:?}", report.fatal);
    assert!(report.fatal[0].ends_with("is outside the project directory, the outDir and the include path"));
    assert!(!project.out_exists());
    assert!(!project.root.join("probe-report.json").exists());
}

#[test]
fn declining_after_lowering_is_an_error_and_a_call_is_offered_once() {
    let Some(project) = Project::new("decline") else { return };
    project.write(
        "src/shared/declines.ts",
        "import { helper } from \"./macros\";\nexport const a = probeDeclineAfter(1);\nexport const b = probeHoistDecline();\nexport const c = probePrereqDecline();\n\
         export const d = probeArgDecline(probeHoist(sideEffect()));\nexport const e = void probeVoidHoist;\n\
         export const f = probeArgDecline(probeSelf(1));\n\
         export const g = capMap[probePrereq(tostring(sideEffect()))].cr(1);\n\
         export const h = void void probeVoidSelf;\nexport const i = probeLateDecline();\n\
         export const j = probeMarkDecline(helper);\nexport const k = probeTwice(1);\nprobeStmtSelf;\n",
    );
    let report = project.build(&["stamp", "probe", "probe-exprcall"], |_| {});
    let all = diagnostics(&report);
    let declines: Vec<&String> = all.iter().filter(|d| d.starts_with("error declines.ts")).collect();
    assert_eq!(
        declines,
        vec![
            "error declines.ts: rbxts-rs internal error: plugin `probe` declined a call after lowering it with compile_expr",
            "error declines.ts: rbxts-rs internal error: plugin `probe` declined a call after calling hoist",
            "error declines.ts: rbxts-rs internal error: plugin `probe` declined a call after emitting prerequisites (they would run twice)",
            // the hoist of the nested probeHoist claim, made while the argument was compiled, would be made twice
            "error declines.ts: rbxts-rs internal error: plugin `probe` declined a call after leaving statements before \
             the root statement behind (a hoist or an insertion, its own or a nested hook's)",
            "error declines.ts: rbxts-rs internal error: plugin `probe` declined an expression after calling hoist",
            // the nested probeSelf claim would run again
            "error declines.ts: rbxts-rs internal error: plugin `probe` declined a call after a node inside it was \
             offered to a plugin hook (it would be offered again)",
            // the receiver's prereqs were captured and dropped: it is not reused
            "error declines.ts: rbxts-rs internal error: plugin `probe` declined a call after compiling the receiver \
             inside a capture",
            // the inner `void probeVoidSelf` hook ran inside the outer one: its frame does not reset the outer's
            "error declines.ts: rbxts-rs internal error: plugin `probe` declined an expression after lowering it with \
             compile_expr",
            "error declines.ts: rbxts-rs internal error: plugin `probe` declined a call after calling late_string",
            "error declines.ts: rbxts-rs internal error: plugin `probe` declined a call after calling mark_import_used",
            "error declines.ts: rbxts-rs internal error: plugin `probe`: compiled its own call a second time (it would be \
             lowered twice)",
            "error declines.ts: rbxts-rs internal error: plugin `probe-exprcall`: a statement hook cannot compile its own \
             statement (compile_* takes expressions; lower the statement's parts)",
        ]
    );
    // the hook that compiled the call itself and declined ran once: the call is not offered again
    assert_eq!(all.iter().filter(|d| d.ends_with("probe: declineafter was offered")).count(), 1, "{all:?}");
    assert!(!project.out_exists());
}

#[test]
fn an_erroneous_decline_ends_the_offer() {
    let Some(project) = Project::new("decline-ends") else { return };
    project.write("src/shared/declines.ts", "export const a = probeDeclineAfter(1);\n");
    let report = project.build(&["stamp", "probe", "probe-b"], |_| {});
    let all = diagnostics(&report);
    // probe declined after lowering the call: an error, and probe-b is not asked (it would warn too)
    assert_eq!(all.iter().filter(|d| d.ends_with("probe: declineafter was offered")).count(), 1, "{all:?}");
    assert_eq!(
        errors(&report),
        vec![
            "error declines.ts: rbxts-rs internal error: plugin `probe` declined a call after lowering it with compile_expr"
        ]
    );
}

#[test]
fn claims_all_never_changes_the_output() {
    let Some(project) = Project::new("claims-all") else { return };
    // without the switch: probe-touched is only asked where probe declined, and changes nothing
    let report = project.build(&["stamp", "probe", "probe-touched"], |_| {});
    assert_eq!(errors(&report), Vec::<String>::new());
    assert_expected_except(&report, &["shared/exprcall.luau"]);
    // with it: probe-touched is asked about calls probe took, and its hook changes compiler state before declining,
    // so the build fails instead of writing different output
    std::fs::remove_dir_all(project.root.join("out")).unwrap();
    let report = project.build(&["stamp", "probe", "probe-touched"], |o| o.claims_all = true);
    let touched: Vec<String> = errors(&report).into_iter().filter(|e| e.contains("changed compiler state")).collect();
    assert!(!touched.is_empty(), "{:?}", errors(&report));
    assert!(touched.iter().all(|e| e.ends_with(
        "plugin `probe-touched` changed compiler state when asked about a call plugin `probe` claimed \
         (RBXTS_PLUGIN_CLAIMS=all fails such a build rather than change its output)"
    )));
    assert!(!project.out_exists());
}

#[test]
fn call_shapes_and_interests_s1_does_not_dispatch_fail_clearly() {
    let Some(project) = Project::new("s2") else { return };
    project.write(
        "src/shared/s2.ts",
        "export const o = marked.probeMethod?.(1);\n\
         export class Sub extends MarkedBase {\n\
         \tprobeMethod(value: number) {\n\
         \t\treturn super.probeMethod(value);\n\
         \t}\n\
         }\n",
    );
    let report = project.build(&["stamp", "probe"], |_| {});
    assert_eq!(
        errors(&report),
        vec![
            "error s2.ts: rbxts-rs internal error: plugin `probe`: this call matches its interest, but optional method \
             calls (`o.m?.()`, `o[k]?.()`) are not offered to plugins until S2",
            "error s2.ts: rbxts-rs internal error: plugin `probe`: this call matches its interest, but `super.m()` calls are \
             not offered to plugins until S2",
        ]
    );
    assert!(!project.out_exists());

    std::fs::remove_file(project.root.join("src/shared/s2.ts")).unwrap();
    let report = project.build(&["stamp", "probe-s2-interest"], |_| {});
    assert_eq!(
        report.fatal,
        vec![
            "rbxts-rs internal error: plugin `probe` registered accesses_by_marker (the property_key hook), which is not \
             supported until S2"
        ]
    );
    assert!(!project.out_exists());
}

#[test]
fn plugin_panics_outside_the_transform_fail_the_build() {
    let Some(project) = Project::new("panics") else { return };
    let report = project.build(&["stamp", "probe-setup-panic"], |_| {});
    assert_eq!(report.fatal, vec!["plugin `probe` panicked in setup: probe: setup panic"]);
    let report = project.build(&["stamp", "probe-finish-panic"], |_| {});
    assert_eq!(report.fatal, vec!["plugin `probe` panicked in finish: probe: finish panic"]);
    assert!(!project.out_exists());
}

#[test]
fn artifacts_cannot_escape_through_links_or_alias_a_luau_output() {
    let Some(project) = Project::new("sneaky") else { return };
    let outside = std::env::temp_dir().join(format!("rbxts-rs-plugins-outside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    let link = project.root.join("link");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "could not make the junction");
    let report = project.build(&["stamp", "probe-artifact-sneaky"], |_| {});
    let _ = std::process::Command::new("cmd").args(["/C", "rmdir"]).arg(&link).status();
    assert_eq!(report.fatal.len(), 2, "{:?}", report.fatal);
    assert!(
        report.fatal[0].ends_with("calls.luau. has a path component ending in `.` or a space"),
        "{:?}",
        report.fatal
    );
    assert!(
        report.fatal[1]
            .ends_with("through-the-junction.txt is outside the project directory, the outDir and the include path"),
        "{:?}",
        report.fatal
    );
    assert!(!project.out_exists());
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "nothing written through the junction");
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn checker_errors_are_explained_by_plugins() {
    let Some(project) = Project::new("explain") else { return };
    project.write("src/shared/broken.ts", "export const z = probeBrokenName();\n");
    let report = project.build(&["stamp", "probe"], |_| {});
    let messages: Vec<String> = report.diagnostics.iter().map(|d| flatten_diagnostic_message(*d, "\n")).collect();
    assert_eq!(messages, vec!["probe explained: Cannot find name 'probeBrokenName'.\nDeclare probeBroken first."]);
    assert!(report.files.is_empty(), "nothing is transformed after a failed check");
    assert!(!project.out_exists());
}

#[test]
fn tsconfig_plugins_must_be_compiled_in() {
    let Some(project) = Project::new("activation") else { return };
    let report = project.build(&[], |_| {});
    assert_eq!(
        report.fatal,
        vec![
            "tsconfig plugin \"rbxts-transform-stamp\" is not compiled into this toolchain (no registered plugin has \
             that tsconfig_transform)"
        ]
    );
    // a plugin with a tsconfig_transform is off without its entry
    project.write(
        "tsconfig.json",
        &std::fs::read_to_string(fixture_dir().join("tsconfig.json")).unwrap().replace(
            ",\n\t\t\"plugins\": [{ \"transform\": \"rbxts-transform-stamp\", \"prefix\": \"fixture \" }]",
            "",
        ),
    );
    let report = project.build(&["stamp"], |_| {});
    assert!(report.plugins.is_empty(), "{:?}", report.plugins);
    // and the build without plugins compiles the plugin calls as plain calls (the `debugger` is roblox-ts's error)
    assert_eq!(errors(&report).len(), 1, "{:?}", errors(&report));
    assert!(
        errors(&report)[0].starts_with("error fallbacks.ts: `debugger` is not supported!"),
        "{:?}",
        errors(&report)
    );
}
