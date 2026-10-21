//! The Flamework plugin end to end, through the real compiler (`rbxts_transform::compile_project_with`) on
//! `tests/fixture`: class metadata, ids in every mode, path and glob macros, `flamework.build` and the include JSON.
//!
//! `tests/fixture/expected` holds a full-mode build, checked against the TS5 pipeline (roblox-ts master with the
//! monorepo transformer at 1a72fa1) on the same sources: equal but for `main.server.luau` (`Flamework.id` is F2,
//! left as a plain call), `namespaced.luau` (TS7 sorts the exports), `flamework.build` (the identifiers follow
//! the program order, where roblox-ts transforms files in `getChangedFilePaths` order; glob matches are sorted,
//! Q-F4) and `globs.json` (the sorted matches).
//!
//! The fixture is copied to a temporary directory with a `node_modules` made as an install would have it: the
//! `@rbxts` packages copied (`FLAMEWORK_RS_RBXTS`, else the real project's, whose compiler-types both
//! roblox-ts master and rbxts-rs accept), and the built `core`, `components` and
//! `networking` packages (`out`, `package.json`, `flamework.build`) copied from the monorepo
//! (`FLAMEWORK_RS_MONOREPO`, else `E:/Projects/TS/Flamework/ExperimentalV2Monorepo`). A junction to the packages
//! themselves would take the Rojo resolver into their `node_modules`, whose bun links are relative and break
//! through a junction. Without a monorepo the tests are skipped, saying so.

#![cfg(windows)]

use std::path::{Path, PathBuf};

use flamework_rs::Flamework;
use flamework_rs::json::{self, JsValue};
use rbxts_plugin::erased::{DynPlugin, erase};
use rbxts_project::{ProjectOptionLayer, STACK_SIZE};
use rbxts_transform::{CompileOptions, CompileReport, PluginOptions, compile_project_with};
use tsrs_compiler::diagnosticwriter::flatten_diagnostic_message;
use tsrs_diagnostics::Category;

const DEFAULT_MONOREPO: &str = "E:/Projects/TS/Flamework/ExperimentalV2Monorepo";

/// The `@rbxts` packages the fixture needs.
const RBXTS_PACKAGES: [&str; 3] = ["compiler-types", "t", "types"];

const DEFAULT_RBXTS: &str = "E:/Projects/TS/Flamework/roblox-ts-test/realproject/node_modules/@rbxts";

/// The monorepo with a built core, and a folder with the `@rbxts` packages.
fn sources() -> Option<(PathBuf, PathBuf)> {
    let var = |name: &str, default: &str| {
        std::env::var_os(name).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(default))
    };
    let monorepo = var("FLAMEWORK_RS_MONOREPO", DEFAULT_MONOREPO);
    let rbxts = var("FLAMEWORK_RS_RBXTS", DEFAULT_RBXTS);
    (monorepo.join("packages/core/out/index.d.ts").is_file()
        && RBXTS_PACKAGES.iter().all(|name| rbxts.join(name).is_dir()))
    .then_some((monorepo, rbxts))
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture")
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

/// A copy of the fixture in a temporary directory, removed on drop.
struct Project {
    root: PathBuf,
}

impl Project {
    fn new(label: &str) -> Option<Project> {
        let Some((monorepo, rbxts)) = sources() else {
            eprintln!(
                "skipped: no monorepo with a built core, or no @rbxts packages (set FLAMEWORK_RS_MONOREPO, FLAMEWORK_RS_RBXTS)"
            );
            return None;
        };
        let root = std::env::temp_dir().join(format!("flamework-rs-e2e-{label}-{}", std::process::id()));
        let project = Project { root };
        project.clean();
        let fixture = fixture_dir();
        for name in ["src", "tsconfig.json", "default.project.json", "package.json", "flamework.config.json"] {
            let from = fixture.join(name);
            if from.is_dir() {
                copy_dir(&from, &project.root.join(name));
            } else {
                std::fs::copy(&from, project.root.join(name)).unwrap();
            }
        }
        let node_modules = project.root.join("node_modules");
        for name in RBXTS_PACKAGES {
            copy_dir(&rbxts.join(name), &node_modules.join("@rbxts").join(name));
        }
        for name in ["core", "components", "networking"] {
            let from = monorepo.join("packages").join(name);
            let to = node_modules.join("@flamework-experimental").join(name);
            copy_dir(&from.join("out"), &to.join("out"));
            std::fs::copy(from.join("package.json"), to.join("package.json")).unwrap();
            // The packages as this transformer's version built them: the monorepo may already be at the next
            // release, which the version check would refuse (it has its own test).
            let build = std::fs::read_to_string(from.join("flamework.build")).unwrap();
            let marker = "\"flameworkVersion\": \"";
            let start = build.find(marker).expect("a flameworkVersion") + marker.len();
            let end = start + build[start..].find('"').unwrap();
            let build = format!("{}{}{}", &build[..start], flamework_rs::VERSION, &build[end..]);
            std::fs::write(to.join("flamework.build"), build).unwrap();
        }
        Some(project)
    }

    fn clean(&self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }

    fn write(&self, relative: &str, text: &str) {
        std::fs::write(self.root.join(relative), text).unwrap();
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.root.join(relative)).unwrap_or_else(|e| panic!("{relative}: {e}"))
    }

    fn build(&self) -> CompileReport {
        let plugins: Vec<Box<dyn DynPlugin>> = vec![erase(Flamework::new())];
        let cwd = self.root.to_string_lossy().into_owned();
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(STACK_SIZE)
                .spawn_scoped(scope, move || {
                    compile_project_with(
                        &CompileOptions {
                            tsconfig: "tsconfig.json".into(),
                            cwd,
                            project_options: ProjectOptionLayer {
                                optimized_loops: Some(true),
                                allow_comment_directives: Some(true),
                                ..ProjectOptionLayer::default()
                            },
                            out_dir_override: None,
                            write_partial: false,
                            no_write: false,
                        },
                        &PluginOptions { plugins: &plugins, ..PluginOptions::default() },
                    )
                })
                .unwrap()
                .join()
                .unwrap()
        })
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

fn assert_clean(report: &CompileReport) {
    assert!(report.fatal.is_empty(), "{:?}", report.fatal);
    let errors: Vec<String> = diagnostics(report).into_iter().filter(|d| d.starts_with("error")).collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

/// The Luau of an output file, by its path under `out`.
fn luau<'r>(report: &'r CompileReport, relative: &str) -> &'r str {
    report
        .files
        .iter()
        .find(|f| f.out_path.replace('\\', "/").ends_with(&format!("/out/{relative}")))
        .and_then(|f| f.luau.as_deref())
        .unwrap_or_else(|| panic!("no output {relative}"))
}

/// The full-mode build equals `tests/fixture/expected`: every Luau file, `flamework.build` and the include JSON.
#[test]
fn full_mode_matches_the_expected_outputs() {
    let Some(project) = Project::new("full") else { return };
    let report = project.build();
    assert_clean(&report);
    assert_eq!(report.plugins, vec!["flamework"]);

    let expected = fixture_dir().join("expected");
    let mut failures = Vec::new();
    for file in &report.files {
        let relative = Path::new(&file.out_path)
            .strip_prefix(&report.out_dir)
            .unwrap_or(Path::new(&file.out_path))
            .to_string_lossy()
            .replace('\\', "/");
        let Some(luau) = &file.luau else {
            failures.push(format!("{relative}: no output"));
            continue;
        };
        let want = std::fs::read_to_string(expected.join("out").join(&relative))
            .unwrap_or_else(|e| panic!("{relative}: {e}"))
            .replace("\r\n", "\n");
        if *luau != want {
            failures.push(format!("{relative}: differs from expected\n--- ours\n{luau}"));
        }
    }
    for artifact in ["flamework.build", "include/flamework/globs.json", "include/flamework/paths.json", "include/flamework/config.json"] {
        let want = std::fs::read_to_string(expected.join(artifact)).unwrap().replace("\r\n", "\n");
        let ours = project.read(artifact);
        if ours != want {
            failures.push(format!("{artifact}: differs from expected\n--- ours\n{ours}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    // The warnings: the glob that matches nothing, and the F2 macro left as a plain call.
    let warnings = report.warnings.join("\n");
    assert!(
        warnings.contains(
            "src/server/main.server.ts:14:26: flamework: the glob 'src/server/nothing/*.ts' given to registerProvidersGlob matches no files"
        ),
        "{warnings}"
    );
    assert!(warnings.contains("1 Flamework macro call(s) need features rbxts-rs does not have yet (F2: 1); they compile as plain calls: id: the intrinsic-inline parameter intrinsic (1)"), "{warnings}");
}

/// The keys of `flamework.build`, in order.
fn keys(text: &str) -> Vec<String> {
    match json::parse(text).unwrap() {
        JsValue::Object(object) => object.keys().cloned().collect(),
        _ => panic!("not an object"),
    }
}

/// `short` ids are late strings, assigned in program order in `finish`: the Luau and `flamework.build` agree, a
/// second build of the same sources gives the same bytes (but for the salt, which every clean build rotates), and
/// the key order is DESIGN §10.4's.
#[test]
fn short_ids_are_late_and_consistent() {
    let Some(project) = Project::new("short") else { return };
    project.write(".env", "F1_IDMODE=short\n");
    let report = project.build();
    assert_clean(&report);

    let build = project.read("flamework.build");
    assert_eq!(
        keys(&build),
        ["version", "flameworkVersion", "identifiers", "inputs", "idGenerationMode", "salt", "metadata"]
    );
    let info = json::parse(&build).unwrap();
    let identifiers = info.get("identifiers").and_then(JsValue::as_object).unwrap();
    // The new ids, in program order (`finish` assigns the counters walking the parts in it).
    let ids: Vec<(String, String)> =
        identifiers.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned())).collect();
    assert!(!ids.is_empty());
    for (internal, id) in &ids {
        // `<file>@<name>{<hash>}`, the file `init` for an index file.
        let name = internal.rsplit('@').next().unwrap();
        assert!(id.contains(&format!("@{name}{{")), "{internal} -> {id}");
        if internal.contains("/hub/init@") {
            assert!(id.starts_with("init@"), "{id}");
        }
    }
    // The Luau carries the same ids.
    let alpha = luau(&report, "server/services/alpha.luau");
    let alpha_id = &ids.iter().find(|(k, _)| k.ends_with("@Alpha")).unwrap().1;
    assert!(alpha.contains(&format!("\"identifier\", \"{alpha_id}\"")), "{alpha}");

    // The same sources build to the same ids with the same salt: pin it through the config.
    let salt = info.get("salt").and_then(JsValue::as_str).unwrap().to_owned();
    project.write(
        "flamework.config.json",
        &format!("{{ \"transformer\": {{ \"idGenerationMode\": \"short\", \"salt\": \"{salt}\" }} }}"),
    );
    let again = project.build();
    assert_clean(&again);
    assert_eq!(again.files.len(), report.files.len());
    for (first, second) in report.files.iter().zip(&again.files) {
        assert_eq!(first.luau, second.luau, "{}", first.out_path);
    }
    let again_build = project.read("flamework.build");
    let again_info = json::parse(&again_build).unwrap();
    assert_eq!(again_info.get("identifiers"), info.get("identifiers"));
}

/// Obfuscated builds hash the glob text in the code and in `globs.json` alike (with a pinned seed they are
/// reproducible), and the ids are hashes without a file or name.
#[test]
fn obfuscated_globs_and_ids() {
    let Some(project) = Project::new("obfuscated") else { return };
    project.write(".env", "F1_OBFUSCATE=true\nF1_IDMODE=obfuscated\n");
    let report = project.build();
    assert_clean(&report);
    let main = luau(&report, "server/main.server.luau");
    let globs = project.read("include/flamework/globs.json");
    let parsed = json::parse(&globs).unwrap();
    let game = parsed.get("game").and_then(JsValue::as_object).unwrap();
    assert_eq!(game.len(), 3, "{globs}");
    for (key, _) in game.iter() {
        assert!(flamework_rs::uuid::validate(key), "{key}");
        assert!(main.contains(&format!("\"{key}\"")), "{main}");
    }
    let build = project.read("flamework.build");
    assert_eq!(
        keys(&build),
        [
            "version",
            "flameworkVersion",
            "identifiers",
            "inputs",
            "idGenerationMode",
            "salt",
            "buildSeed",
            "metadata"
        ]
    );
}

/// A user's class that asks for guards is refused, naming the milestone, rather than given partial metadata.
#[test]
fn guards_are_refused_until_f3() {
    let Some(project) = Project::new("guards") else { return };
    project.write(
        "src/server/guarded.ts",
        "/** @metadata reflect flamework:guard */\nexport function Guarded() {\n\treturn (t: object, k: string) => print(t, k);\n}\n\nexport class G {\n\t@Guarded()\n\tvalue: number = 1;\n}\n",
    );
    let report = project.build();
    let all = diagnostics(&report);
    assert!(
        all.iter().any(|d| d
            == "error guarded.ts: flamework:guard needs guard generation, which rbxts-rs's Flamework does not have yet (F3)."),
        "{all:#?}"
    );
    // A build with errors writes nothing.
    assert!(!project.root.join("flamework.build").exists());
}

/// A Flamework package built by another version is refused before anything is transformed.
#[test]
fn mixed_package_versions_are_refused() {
    let Some(project) = Project::new("versions") else { return };
    // core: the fixture imports it, so its flamework.build is found above the program's files (components is not).
    let path = "node_modules/@flamework-experimental/core/flamework.build";
    let build = project.read(path).replace(
        &format!("\"flameworkVersion\": \"{}\"", flamework_rs::VERSION),
        "\"flameworkVersion\": \"2.0.0-alpha.1\"",
    );
    project.write(path, &build);
    let report = project.build();
    let fatal = report.fatal.join("\n");
    assert!(fatal.contains("Flamework's packages are not all the same version"), "{fatal}");
    assert!(fatal.contains("@flamework-experimental/core 2.0.0-alpha.1"), "{fatal}");
    assert!(!project.root.join("out").exists());
}
