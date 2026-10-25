//! A fixture copied to a temporary directory with a `node_modules` as an install has it, built with the real
//! compiler (`compile_project_with`). As `tests/e2e.rs` does it: the `@rbxts` packages from `FLAMEWORK_RS_RBXTS`
//! (else the real project's), the built Flamework packages from `FLAMEWORK_RS_MONOREPO` (else the monorepo), their
//! `flamework.build` set to this transformer's version. Without them the tests are skipped, saying so.

use std::path::{Path, PathBuf};

use flamework_rs::Flamework;
use rbxts_plugin::erased::{DynPlugin, erase};
use rbxts_project::{ProjectOptionLayer, STACK_SIZE};
use rbxts_transform::{CompileOptions, CompileReport, PluginOptions, compile_project_with};
use tsrs_compiler::diagnosticwriter::flatten_diagnostic_message;
use tsrs_diagnostics::Category;

const DEFAULT_MONOREPO: &str = "E:/Projects/TS/Flamework/ExperimentalV2Monorepo";
const DEFAULT_RBXTS: &str = "E:/Projects/TS/Flamework/roblox-ts-test/realproject/node_modules/@rbxts";
const RBXTS_PACKAGES: [&str; 3] = ["compiler-types", "t", "types"];

fn sources() -> Option<(PathBuf, PathBuf)> {
    let var = |name: &str, default: &str| std::env::var_os(name).map(PathBuf::from).unwrap_or_else(|| default.into());
    let monorepo = var("FLAMEWORK_RS_MONOREPO", DEFAULT_MONOREPO);
    let rbxts = var("FLAMEWORK_RS_RBXTS", DEFAULT_RBXTS);
    (monorepo.join("packages/core/out/index.d.ts").is_file() && RBXTS_PACKAGES.iter().all(|n| rbxts.join(n).is_dir()))
        .then_some((monorepo, rbxts))
}

pub fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(name)
}

pub fn copy_dir(from: &Path, to: &Path) {
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

/// A copy of a fixture in a temporary directory, removed on drop.
pub struct Project {
    pub root: PathBuf,
}

impl Project {
    /// A copy of `tests/<fixture>` (its sources and config files, not `expected`).
    pub fn new(fixture: &str, label: &str) -> Option<Project> {
        let Some((monorepo, rbxts)) = sources() else {
            eprintln!(
                "skipped: no monorepo with a built core, or no @rbxts packages (set FLAMEWORK_RS_MONOREPO, FLAMEWORK_RS_RBXTS)"
            );
            return None;
        };
        let root = std::env::temp_dir().join(format!("flamework-rs-{fixture}-{label}-{}", std::process::id()));
        let project = Project { root };
        project.clean();
        let from = fixture_dir(fixture);
        for name in ["src", "tsconfig.json", "default.project.json", "package.json", "flamework.config.json", ".env"] {
            let source = from.join(name);
            if source.is_dir() {
                copy_dir(&source, &project.root.join(name));
            } else if source.is_file() {
                std::fs::copy(&source, project.root.join(name)).unwrap();
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

    pub fn write(&self, relative: &str, text: &str) {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    pub fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.root.join(relative)).unwrap_or_else(|e| panic!("{relative}: {e}"))
    }

    pub fn exists(&self, relative: &str) -> bool {
        self.root.join(relative).exists()
    }

    /// Builds with `flamework` (a fresh one per build: its macro types).
    pub fn build_with(&self, flamework: Flamework) -> CompileReport {
        let plugins: Vec<Box<dyn DynPlugin>> = vec![erase(flamework)];
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
                            checkers: None,
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
pub fn diagnostics(report: &CompileReport) -> Vec<String> {
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

pub fn errors(report: &CompileReport) -> Vec<String> {
    let mut errors: Vec<String> = diagnostics(report).into_iter().filter(|d| d.starts_with("error")).collect();
    errors.extend(report.fatal.iter().cloned());
    errors
}

pub fn assert_clean(report: &CompileReport) {
    let errors = errors(report);
    assert!(errors.is_empty(), "{errors:#?}");
}

/// The Luau of an output file, by its path under `out`.
pub fn luau<'r>(report: &'r CompileReport, relative: &str) -> &'r str {
    report
        .files
        .iter()
        .find(|f| f.out_path.replace('\\', "/").ends_with(&format!("/out/{relative}")))
        .and_then(|f| f.luau.as_deref())
        .unwrap_or_else(|| panic!("no output {relative}"))
}

/// Every output and artifact against `tests/<fixture>/expected`; the differences, described.
pub fn compare_expected(project: &Project, report: &CompileReport, fixture: &str) -> Vec<String> {
    let expected = fixture_dir(fixture).join("expected");
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
        let path = expected.join(artifact);
        match (path.is_file(), project.exists(artifact)) {
            (false, false) => {}
            (true, true) => {
                let want = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
                let ours = project.read(artifact);
                if ours != want {
                    failures.push(format!("{artifact}: differs from expected\n--- ours\n{ours}"));
                }
            }
            (want, ours) => failures.push(format!("{artifact}: expected {want}, written {ours}")),
        }
    }
    failures
}
