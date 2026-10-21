//! The build pipeline (`Project/functions/compileFiles.ts`, DESIGN §4): load, check every project file, set up the
//! project and the plugins, transform each file in program order, let the plugins finish, render, and commit. The
//! lifecycle itself, with incremental and watch builds, is [`crate::build::BuildSession`]; this module has the
//! one-build entry points and the per-file pieces.
//!
//! Any error stops the build before anything is written: checker and setup errors before the transform, transform
//! errors before rendering (roblox-ts's rule), plugin `finish` errors and unresolved late strings before
//! rendering. [`CompileOptions::write_partial`] relaxes the second for parity runs: files that transformed cleanly
//! are written even when other files failed (plugin artifacts never are).
//!
//! **Commit order** (DESIGN §14), only when the build has no errors: Luau outputs and assets; plugin artifacts (only
//! if changed); deletions (stale outputs, stale artifacts, removals); plugin state files, then the incremental
//! state. Every file is written to a temporary file and renamed into place, with retries while another process holds
//! the target ([`crate::plugins::lifecycle::write_atomic`]).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_luau::Stmt;
use rbxts_plugin::erased::{ErasedPart, ErasedShared};
use rbxts_plugin::{CheckerInfo, FileIndex, FileInfo};
use rbxts_project::{Project, ProjectOptionLayer, ProjectServices, nodepath};
use tsrs_ast::{Diagnostic, SourceFile, new_diagnostic};
use tsrs_compiler::Context;
use tsrs_core::P;
use tsrs_diagnostics::Category;

use crate::classes::macro_manager::MacroManager;
use crate::classes::multi_transform_state::MultiTransformState;
use crate::classes::prereqs::reset_temp_ids;
use crate::classes::transform_state::TransformState;
use crate::nodes::transform_source_file::transform_source_file;
use crate::plugins::backend::{run_hook, to_diagnostic};
use crate::plugins::file_state::PluginFileState;
use crate::plugins::host::HookTime;
use crate::plugins::lifecycle::{ActivePlugin, BuildInput, PluginOptions, PluginTimings};
use crate::plugins::{HookKind, PluginHost};
use crate::shared::diagnostics::internal_error;

/// What to build and how.
#[derive(Clone, Debug)]
pub struct CompileOptions {
    /// The tsconfig, absolute or relative to `cwd`.
    pub tsconfig: String,
    pub cwd: String,
    /// Command-line project options (`--optimizedLoops`, `--allowCommentDirectives`, …) over the tsconfig's
    /// `rbxts` section.
    pub project_options: ProjectOptionLayer,
    /// Write outputs here instead of `outDir`, keeping Rojo resolution against the configured `outDir` (so parity
    /// runs do not touch a project's real output).
    pub out_dir_override: Option<String>,
    /// Write the files that transformed without errors even when others failed (parity runs). roblox-ts writes
    /// nothing when any file has an error.
    pub write_partial: bool,
    /// Transform and render, but write nothing.
    pub no_write: bool,
}

/// Wall time per phase.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    pub config: Duration,
    pub program: Duration,
    pub setup: Duration,
    /// File facts and the invalidation rules (incremental and watch builds), with the missing-output checks.
    pub invalidate: Duration,
    pub check: Duration,
    /// Plugin `setup` (all plugins).
    pub plugin_setup: Duration,
    pub transform: Duration,
    /// Plugin `finish` (all plugins) and the late-string key check.
    pub plugin_finish: Duration,
    pub render: Duration,
    pub write: Duration,
}

/// One project file's outcome.
pub struct FileOutcome {
    pub file_name: String,
    /// Where the output goes (after `out_dir_override`).
    pub out_path: String,
    /// The transform's diagnostics (roblox-ts's, rbxts-rs's own, and plugins').
    pub diagnostics: Vec<P<Diagnostic>>,
    /// The kinds reported as not yet supported, in report order (duplicates kept).
    pub unsupported: Vec<String>,
    /// Internal errors (panics caught while transforming).
    pub internal_errors: Vec<String>,
    /// The rendered Luau, when the file transformed without errors.
    pub luau: Option<String>,
    /// The output was written (it was missing or different).
    pub written: bool,
}

impl FileOutcome {
    /// The file transformed without error diagnostics.
    pub fn is_clean(&self) -> bool {
        !self.diagnostics.iter().any(|d| d.category() == Category::Error)
    }
}

/// What a build did.
#[derive(Default)]
pub struct CompileReport {
    pub timings: Timings,
    /// Config, program, setup and checker diagnostics (the transform's are per file).
    pub diagnostics: Vec<P<Diagnostic>>,
    /// Setup warnings (Rojo config search, resolver warnings) and plugin warnings.
    pub warnings: Vec<String>,
    /// A failure that stopped the build, with its messages (roblox-ts's `ProjectError` / text diagnostics, plugin
    /// `Fatal`s, unresolved late strings, commit failures).
    pub fatal: Vec<String>,
    /// One per transformed file, in program order.
    pub files: Vec<FileOutcome>,
    /// Where the outputs go: `outDir`, or the override.
    pub out_dir: String,
    /// Files in the program, and project files transformed.
    pub program_files: usize,
    pub project_files: usize,
    /// The plugins that were on, in registration order.
    pub plugins: Vec<&'static str>,
    /// Plugin hook times (`--timings`).
    pub plugin_timings: Vec<PluginTimings>,
    /// Plugin `info` messages (`--verbose`).
    pub plugin_messages: Vec<String>,
    /// What plugins read in `setup` (rbxts-rs: S2 — watched and hashed by incremental and watch builds).
    pub build_inputs: Vec<BuildInput>,
    /// Plugin artifacts and state files written, and files plugins removed.
    pub artifacts_written: Vec<String>,
    pub artifacts_deleted: Vec<String>,
}

impl CompileReport {
    /// The build failed (nothing written unless `write_partial`).
    pub fn has_errors(&self) -> bool {
        !self.fatal.is_empty()
            || self.diagnostics.iter().any(|d| d.category() == Category::Error)
            || self.files.iter().any(|f| !f.is_clean())
    }
}

/// `fileUsesCommentDirectives`: `@ts-ignore`, `@ts-expect-error` and `@ts-nocheck` need `allowCommentDirectives`.
pub(crate) fn comment_directive_diagnostics(file: P<SourceFile>) -> Vec<P<Diagnostic>> {
    let message = || tsrs_diagnostics::new_ad_hoc_message(&errors_text_no_comment_directives());
    let mut diagnostics = Vec::new();
    for directive in file.get().comment_directives() {
        diagnostics.push(new_diagnostic(Some(file), directive.loc, message(), &[]));
    }
    if let Some(directive) = file.get().check_js_directive()
        && !directive.enabled
    {
        diagnostics.push(new_diagnostic(Some(file), directive.range.text_range, message(), &[]));
    }
    diagnostics
}

fn errors_text_no_comment_directives() -> String {
    // the text of `errors.noCommentDirectives`, which takes a node there
    [
        "Usage of `@ts-ignore`, `@ts-expect-error`, and `@ts-nocheck` are not supported!",
        "roblox-ts needs type and symbol info to compile correctly.",
        "Suggestion: Consider using type assertions or `declare` statements.",
    ]
    .join("\n")
}

/// The output path, moved under `out_dir_override` when there is one.
pub(crate) fn output_path(services: &ProjectServices, file_name: &str, out_dir_override: Option<&str>) -> String {
    let out_path = services.path_translator.get_output_path(file_name);
    match out_dir_override {
        Some(dir) => {
            let relative = nodepath::relative(&services.path_translator.out_dir, &out_path);
            nodepath::join(&[dir, &relative])
        }
        None => out_path,
    }
}

/// The plugin facts of a file.
fn file_info(services: &ProjectServices, host: &PluginHost, file: P<SourceFile>) -> FileInfo {
    let path = file.get().file_name().to_owned();
    let relative_path = nodepath::relative(&services.data.project_path, &path).replace('\\', "/");
    FileInfo {
        source: file,
        index: host.file_indices.get(&file).copied().unwrap_or(FileIndex(u32::MAX)),
        rojo: services.file_info(&path),
        path,
        relative_path,
    }
}

/// The text of a caught panic.
pub(crate) fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "panic".to_owned())
}

/// One file's transform: its Luau (unless it panicked) and what it reported.
pub(crate) struct FileTransform {
    pub ast: Option<Vec<Stmt>>,
    pub diagnostics: Vec<P<Diagnostic>>,
    pub unsupported: Vec<String>,
    pub internal_errors: Vec<String>,
    /// A panic was caught: the checker may be inconsistent, so no further file is transformed.
    pub panicked: bool,
    /// Per plugin: the file's part.
    pub parts: Vec<Option<ErasedPart>>,
    /// The late keys its Luau uses: (plugin, namespaced key).
    pub late_keys: Vec<(usize, String)>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn transform_file(
    project: &Project,
    checker: &mut tsrs_checker::Checker,
    services: &ProjectServices,
    macro_manager: &MacroManager,
    multi: &mut MultiTransformState,
    host: Option<&PluginHost>,
    file: P<SourceFile>,
) -> FileTransform {
    reset_temp_ids();
    let mut state = TransformState::new(project.program, checker, services, macro_manager, multi, file);
    if let Some(host) = host {
        state.plugins = Some(host);
        state.plugin_file = PluginFileState::new(file_info(services, host, file), host.len());
    }
    let result = catch_unwind(AssertUnwindSafe(|| transform_source_file(&mut state, file)));
    let mut internal_errors = Vec::new();
    let mut panicked = false;
    let ast = match result {
        Ok(ast) => Some(ast),
        Err(panic) => {
            panicked = true;
            let message = panic_message(&*panic);
            // a panic inside a hook is the plugin's; inside compiler code (a hook's compile_* included) the compiler's
            let message = match host.and_then(PluginHost::take_panic_frame) {
                Some((plugin, hook)) => {
                    format!("plugin `{}` panicked in {}: {message}", host.expect("a host").name(plugin), hook.label())
                }
                None => message,
            };
            state.add_diagnostic(internal_error(file.as_node(), &message));
            internal_errors.push(message);
            None
        }
    };
    let diagnostics = std::mem::take(&mut state.diagnostics.diagnostics);
    let unsupported = std::mem::take(&mut state.diagnostics.unsupported);
    let parts = std::mem::take(&mut state.plugin_file.parts);
    let late_keys = std::mem::take(&mut state.plugin_file.late_keys);
    FileTransform { ast, diagnostics, unsupported, internal_errors, panicked, parts, late_keys }
}

/// The check failed: each plugin may explain the checker diagnostics of the failing files
/// (`explain_checker_diagnostic`); an explanation replaces the diagnostic.
pub(crate) fn explain_failures(
    report: &mut CompileReport,
    project: &Project,
    services: &ProjectServices,
    active: &[ActivePlugin<'_>],
    shared: &[ErasedShared],
    times: &[[HookTime; HookKind::ALL.len()]],
    ctx: &Context,
) {
    let failing: Vec<P<SourceFile>> = {
        let mut seen = FxHashSet::default();
        report
            .diagnostics
            .iter()
            .filter(|d| d.category() == Category::Error)
            .filter_map(|d| d.file())
            .filter(|f| seen.insert(*f))
            .collect()
    };
    if failing.is_empty() {
        return;
    }
    let mut checker = project.program.get_type_checker(ctx);
    let Ok(macro_manager) = MacroManager::new(&mut checker) else { return };
    let mut multi = MultiTransformState::new();
    let host = PluginHost::new(
        project.program,
        active,
        shared,
        CheckerInfo { index: 0, count: 1 },
        FxHashMap::default(),
        false,
    );
    let mut replacements: FxHashMap<P<Diagnostic>, P<Diagnostic>> = FxHashMap::default();
    let mut extra = Vec::new();
    for file in failing {
        let diagnostics: Vec<P<Diagnostic>> =
            report.diagnostics.iter().copied().filter(|d| d.file() == Some(file)).collect();
        let mut state = TransformState::new(project.program, &mut checker, services, &macro_manager, &mut multi, file);
        state.plugins = Some(&host);
        state.plugin_file = PluginFileState::new(file_info(services, &host, file), host.len());
        let root = file.as_node();
        for diagnostic in diagnostics {
            for plugin in 0..host.len() {
                let explained = catch_unwind(AssertUnwindSafe(|| {
                    run_hook(&mut state, None, None, plugin, HookKind::Explain, root, |instance, cx| {
                        Ok(instance.explain_checker_diagnostic(cx, diagnostic))
                    })
                }));
                let explained = match explained {
                    Ok(explained) => explained,
                    Err(panic) => {
                        host.take_panic_frame();
                        report.fatal.push(format!(
                            "plugin `{}` panicked in explain_checker_diagnostic: {}",
                            host.name(plugin),
                            panic_message(&*panic)
                        ));
                        continue;
                    }
                };
                if let Ok(Some(explanation)) = explained {
                    replacements.insert(diagnostic, to_diagnostic(&explanation));
                    break;
                }
            }
        }
        extra.extend(std::mem::take(&mut state.diagnostics.diagnostics));
    }
    for diagnostic in &mut report.diagnostics {
        if let Some(&replacement) = replacements.get(diagnostic) {
            *diagnostic = replacement;
        }
    }
    report.diagnostics.extend(extra);
    let hook_times = host.times.borrow();
    record_timings(report, active, times, Some(&hook_times));
}

pub(crate) fn record_timings(
    report: &mut CompileReport,
    active: &[ActivePlugin<'_>],
    outside: &[[HookTime; HookKind::ALL.len()]],
    hooks: Option<&Vec<[HookTime; HookKind::ALL.len()]>>,
) {
    report.plugin_timings = active
        .iter()
        .enumerate()
        .map(|(i, plugin)| PluginTimings {
            name: plugin.info.name,
            hooks: HookKind::ALL
                .iter()
                .map(|&kind| {
                    let mut time = outside[i][kind.index()];
                    if let Some(hooks) = hooks {
                        time.calls += hooks[i][kind.index()].calls;
                        time.time += hooks[i][kind.index()].time;
                    }
                    (kind, time)
                })
                .collect(),
        })
        .collect();
}

/// Builds a project without plugins: check → transform → render → write (DESIGN §4). Runs on the calling thread,
/// which needs a large stack ([`rbxts_project::STACK_SIZE`]).
pub fn compile_project(options: &CompileOptions) -> CompileReport {
    compile_project_with(options, &PluginOptions::default())
}

/// Builds a project with plugins: check → plugin setup → transform (with the plugins' hooks) → plugin finish →
/// render → commit (DESIGN §4, §14), as one build of a [`crate::build::BuildSession`]: a clean build, or an
/// incremental one when the project is `incremental` and its incremental state is current. Runs on the calling
/// thread, which needs a large stack ([`rbxts_project::STACK_SIZE`]).
pub fn compile_project_with(options: &CompileOptions, plugin_options: &PluginOptions<'_>) -> CompileReport {
    let mut session = crate::build::BuildSession::new(options.clone(), plugin_options, false);
    session.build(&[]).0
}
