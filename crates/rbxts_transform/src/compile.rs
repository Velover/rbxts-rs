//! The build pipeline (`Project/functions/compileFiles.ts`, DESIGN §4): load, check every project file, set up the
//! project and the plugins, transform each file in program order, let the plugins finish, render, and commit.
//!
//! Any error stops the build before anything is written: checker and setup errors before the transform, transform
//! errors before rendering (roblox-ts's rule), plugin `finish` errors and unresolved late strings before
//! rendering. [`CompileOptions::write_partial`] relaxes the second for parity runs: files that transformed cleanly
//! are written even when other files failed (plugin artifacts never are).
//!
//! **Commit order** (DESIGN §14), only when the build has no errors: Luau outputs; plugin artifacts (only if
//! changed); removals; plugin state files. Every file is written to a temporary file and renamed into place, with
//! retries while another process holds the target ([`crate::plugins::lifecycle::write_atomic`]).

use std::cell::{OnceCell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::{Duration, Instant};

use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_luau::{LateStrings, Stmt};
use rbxts_plugin::erased::{ErasedPart, ErasedShared, ErasedState};
use rbxts_plugin::{CheckerInfo, FileIndex, FileInfo};
use rbxts_project::{LoadOptions, Project, ProjectOptionLayer, ProjectServices, nodepath};
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
use crate::plugins::lifecycle::{
    ActivePlugin, BuildInput, CommitQueue, Finish, PluginOptions, PluginTimings, Setup, activate, commit_plugin_files,
    output_key, plain_late_key, run_finish, run_setup, write_if_changed,
};
use crate::plugins::{HookKind, PluginHost};
use crate::shared::diagnostics::{internal_error, text_diagnostic};

/// What to build and how.
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
fn comment_directive_diagnostics(file: P<SourceFile>) -> Vec<P<Diagnostic>> {
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
fn output_path(services: &ProjectServices, file_name: &str, out_dir_override: Option<&str>) -> String {
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

/// One file's transform: its Luau (unless it panicked) and what it reported.
struct FileTransform {
    ast: Option<Vec<Stmt>>,
    diagnostics: Vec<P<Diagnostic>>,
    unsupported: Vec<String>,
    internal_errors: Vec<String>,
    /// A panic was caught: the checker may be inconsistent, so no further file is transformed.
    panicked: bool,
    /// Per plugin: the file's part.
    parts: Vec<Option<ErasedPart>>,
    /// The late keys its Luau uses: (plugin, namespaced key).
    late_keys: Vec<(usize, String)>,
}

#[allow(clippy::too_many_arguments)]
fn transform_file(
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
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "panic".to_owned());
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

/// The plugins' setup state, one entry per active plugin.
struct PluginRun<'p> {
    active: Vec<ActivePlugin<'p>>,
    shared: Vec<ErasedShared>,
    states: Vec<ErasedState>,
    times: Vec<[HookTime; HookKind::ALL.len()]>,
}

/// Runs every active plugin's `setup`. A `Fatal` stops the build.
fn setup_plugins<'p>(
    report: &mut CompileReport,
    project: &Project,
    services: &ProjectServices,
    active: Vec<ActivePlugin<'p>>,
) -> Option<PluginRun<'p>> {
    let resolver = OnceCell::new();
    let mut run = PluginRun {
        shared: Vec::with_capacity(active.len()),
        states: Vec::with_capacity(active.len()),
        times: vec![[HookTime::default(); HookKind::ALL.len()]; active.len()],
        active: Vec::new(),
    };
    for (index, plugin) in active.iter().enumerate() {
        let mut backend = Setup {
            program: project.program,
            services,
            plugin: plugin.info.name,
            entry: plugin.entry.as_ref(),
            resolver: &resolver,
            inputs: RefCell::default(),
            messages: &mut report.plugin_messages,
            warnings: &mut report.warnings,
        };
        let (shared, state, time) = run_setup(plugin, &mut backend);
        let inputs = backend.inputs.into_inner();
        report.build_inputs.extend(inputs);
        PluginHost::add_time(&mut run.times, index, HookKind::Setup, time);
        match shared {
            Ok(shared) => {
                run.shared.push(shared);
                run.states.push(state);
            }
            Err(fatal) => report.fatal.push(format!("plugin `{}`: {fatal}", plugin.info.name)),
        }
    }
    run.active = active;
    report.fatal.is_empty().then_some(run)
}

/// The check failed: each plugin may explain the checker diagnostics of the failing files
/// (`explain_checker_diagnostic`); an explanation replaces the diagnostic.
fn explain_failures(
    report: &mut CompileReport,
    project: &Project,
    services: &ProjectServices,
    run: &PluginRun<'_>,
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
        &run.active,
        &run.shared,
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
                let explained = run_hook(&mut state, None, None, plugin, HookKind::Explain, root, |instance, cx| {
                    Ok(instance.explain_checker_diagnostic(cx, diagnostic))
                });
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
    let times = host.times.borrow();
    record_timings(report, &run.active, &run.times, Some(&times));
}

fn record_timings(
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
/// render → commit (DESIGN §4, §14). Clean builds only (rbxts-rs: S2 — incremental and watch builds). Runs on the
/// calling thread, which needs a large stack ([`rbxts_project::STACK_SIZE`]).
pub fn compile_project_with(options: &CompileOptions, plugin_options: &PluginOptions<'_>) -> CompileReport {
    let mut report = CompileReport::default();

    // load: tsconfig and program
    let project = match rbxts_project::load(&LoadOptions {
        tsconfig: options.tsconfig.clone(),
        cwd: options.cwd.clone(),
        checkers: 1,
        lazy_dts: false,
    }) {
        Ok(project) => project,
        Err(diagnostics) => {
            report.diagnostics = diagnostics;
            report.fatal.push("the tsconfig could not be used".to_owned());
            return report;
        }
    };
    report.timings.config = project.config_time;
    report.timings.program = project.program_time;
    report.program_files = project.program.source_files().len();

    // project setup: options, Rojo, path translator, roblox-ts's pre-transform checks
    let start = Instant::now();
    let tsconfig_path = if nodepath::is_absolute(&options.tsconfig) {
        options.tsconfig.clone()
    } else {
        nodepath::join(&[&options.cwd, &options.tsconfig])
    };
    let setup = match project.setup(&tsconfig_path, &options.project_options) {
        Ok(setup) => setup,
        Err(error) => {
            report.fatal = error.messages;
            report.timings.setup = start.elapsed();
            return report;
        }
    };
    report.timings.setup = start.elapsed();
    report.warnings = setup.services.warnings.clone();
    for d in &setup.diagnostics {
        let category = if d.severity == rbxts_project::Severity::Error { Category::Error } else { Category::Warning };
        report.diagnostics.push(text_diagnostic(&d.message, category));
    }
    if let Some(fatal) = &setup.fatal {
        report.fatal.push(fatal.clone());
        return report;
    }
    let services = setup.services;
    report.out_dir = options.out_dir_override.clone().unwrap_or_else(|| services.path_translator.out_dir.clone());

    // the plugins on for this project (tsconfig `plugins` entries)
    let active = match activate(plugin_options, &tsconfig_path) {
        Ok(active) => active,
        Err(errors) => {
            report.fatal = errors;
            return report;
        }
    };
    report.plugins = active.iter().map(|p| p.info.name).collect();
    let claims_all = plugin_options.claims_all || std::env::var("RBXTS_PLUGIN_CLAIMS").is_ok_and(|v| v == "all");

    // check: every project file, before any transform
    let start = Instant::now();
    let ctx = Context::background();
    let files = project.project_files();
    report.project_files = files.len();
    report.diagnostics.extend(project.program_diagnostics());
    report.diagnostics.extend(project.check_files(&ctx, &files));
    if !services.data.options.allow_comment_directives {
        for &file in &files {
            report.diagnostics.extend(comment_directive_diagnostics(file));
        }
    }
    report.timings.check = start.elapsed();
    let check_failed = report.diagnostics.iter().any(|d| d.category() == Category::Error);

    // plugin setup (also when the check failed: explain_checker_diagnostic needs the instances)
    let start = Instant::now();
    let mut run = if active.is_empty() { None } else { setup_plugins(&mut report, &project, &services, active) };
    report.timings.plugin_setup = start.elapsed();
    if !report.fatal.is_empty() {
        return report;
    }
    if check_failed {
        if let Some(run) = &run {
            explain_failures(&mut report, &project, &services, run, &ctx);
        }
        return report;
    }

    // transform: one checker, files in program order
    let start = Instant::now();
    let mut asts: Vec<Option<Vec<Stmt>>> = Vec::with_capacity(files.len());
    let plugin_count = run.as_ref().map_or(0, |r| r.active.len());
    let mut parts: Vec<Vec<(FileIndex, ErasedPart)>> = (0..plugin_count).map(|_| Vec::new()).collect();
    let mut file_late_keys: Vec<Vec<(usize, String)>> = Vec::with_capacity(files.len());
    let mut hook_times = None;
    {
        let mut checker = project.program.get_type_checker(&ctx);
        let macro_manager = match MacroManager::new(&mut checker) {
            Ok(manager) => manager,
            Err(error) => {
                report.fatal = error.messages;
                return report;
            }
        };
        let host = run.as_ref().map(|run| {
            let mut file_diagnostics: FxHashMap<P<SourceFile>, Vec<P<Diagnostic>>> = FxHashMap::default();
            for &d in &report.diagnostics {
                if let Some(file) = d.file() {
                    file_diagnostics.entry(file).or_default().push(d);
                }
            }
            PluginHost::new(
                project.program,
                &run.active,
                &run.shared,
                CheckerInfo { index: 0, count: 1 },
                file_diagnostics,
                claims_all,
            )
        });
        let mut multi = MultiTransformState::new();
        for &file in &files {
            let transformed =
                transform_file(&project, &mut checker, &services, &macro_manager, &mut multi, host.as_ref(), file);
            let file_name = file.get().file_name().to_owned();
            let out_path = output_path(&services, &file_name, options.out_dir_override.as_deref());
            if let Some(host) = &host {
                let index = host.file_indices[&file];
                for (plugin, part) in transformed.parts.into_iter().enumerate() {
                    if let Some(part) = part {
                        parts[plugin].push((index, part));
                    }
                }
            }
            file_late_keys.push(transformed.late_keys);
            report.files.push(FileOutcome {
                file_name: file_name.clone(),
                out_path,
                diagnostics: transformed.diagnostics,
                unsupported: transformed.unsupported,
                internal_errors: transformed.internal_errors,
                luau: None,
                written: false,
            });
            asts.push(transformed.ast);
            if transformed.panicked {
                // rbxts-rs: after a panic the single checker may be inconsistent (FOLLOWUPS T1): stop here
                report.fatal.push(format!(
                    "the transform stopped after an internal error in {file_name}; later files were not transformed"
                ));
                break;
            }
        }
        if let Some(host) = &host {
            hook_times = Some(host.times.borrow().clone());
        }
    }
    report.timings.transform = start.elapsed();

    let any_errors = report.has_errors();
    if any_errors && !options.write_partial {
        if let Some(run) = &run {
            record_timings(&mut report, &run.active, &run.times, hook_times.as_ref());
        }
        return report;
    }

    // plugin finish: every file's part, in program order; then the late-string key check
    let start = Instant::now();
    let mut late_strings = LateStrings::new();
    let mut queue = CommitQueue::default();
    let mut finished = run.is_none();
    if !any_errors && let Some(run) = run.as_mut() {
        let mut luau_outputs = FxHashSet::default();
        for (outcome, &file) in report.files.iter().zip(&files) {
            luau_outputs.insert(output_key(&services, &outcome.out_path));
            luau_outputs
                .insert(output_key(&services, &services.path_translator.get_output_path(file.get().file_name())));
        }
        let mut resolved = FxHashSet::default();
        let mut errors = Vec::new();
        for (index, plugin) in run.active.iter().enumerate() {
            let plugin_parts: Vec<(FileIndex, &ErasedPart)> = parts[index].iter().map(|(i, p)| (*i, p)).collect();
            let mut backend = Finish {
                program: project.program,
                services: &services,
                plugin: plugin.info.name,
                out_dir: &report.out_dir,
                luau_outputs: &luau_outputs,
                late: &mut late_strings,
                resolved: &mut resolved,
                queue: &mut queue,
                errors: &mut errors,
                warnings: &mut report.warnings,
                messages: &mut report.plugin_messages,
            };
            let (result, time) =
                run_finish(plugin, &run.shared[index], &mut run.states[index], &plugin_parts, &mut backend);
            PluginHost::add_time(&mut run.times, index, HookKind::Finish, time);
            if let Err(fatal) = result {
                errors.push(format!("plugin `{}`: {fatal}", plugin.info.name));
            }
        }
        // every late key a file uses must have a value
        let mut reported = FxHashSet::default();
        for (keys, outcome) in file_late_keys.iter().zip(&report.files) {
            for (plugin, key) in keys {
                if !resolved.contains(key) && reported.insert(key.clone()) {
                    let relative =
                        nodepath::relative(&services.data.project_path, &outcome.file_name).replace('\\', "/");
                    errors.push(format!(
                        "plugin `{}` left the late string `{}` unresolved (used in {relative})",
                        run.active[*plugin].info.name,
                        plain_late_key(key)
                    ));
                }
            }
        }
        report.fatal.extend(errors);
        finished = report.fatal.is_empty();
        // rbxts-rs: S2 — on commit, the working states become the committed ones and the Shareds are kept with them
    }
    report.timings.plugin_finish = start.elapsed();
    if let Some(run) = &run {
        record_timings(&mut report, &run.active, &run.times, hook_times.as_ref());
    }
    if !report.fatal.is_empty() && !options.write_partial {
        return report;
    }

    // render (after plugin finish, so late strings are filled in: DESIGN §4 step 6)
    let start = Instant::now();
    for ((outcome, ast), keys) in report.files.iter_mut().zip(asts).zip(&file_late_keys) {
        // a file using late strings renders only when finish resolved them
        if let Some(ast) = ast
            && outcome.is_clean()
            && (keys.is_empty() || finished)
        {
            outcome.luau = Some(rbxts_luau::render(&ast, &late_strings));
        }
    }
    report.timings.render = start.elapsed();

    // commit: Luau (only outputs that changed), then plugin artifacts, removals and state files
    let start = Instant::now();
    if !options.no_write {
        for outcome in &mut report.files {
            let Some(source) = &outcome.luau else { continue };
            match write_if_changed(Path::new(&outcome.out_path), source.as_bytes()) {
                Ok(written) => outcome.written = written,
                Err(e) => report.fatal.push(format!("could not write {}: {e}", outcome.out_path)),
            }
        }
        if report.fatal.is_empty() && !report.has_errors() && finished {
            match commit_plugin_files(&queue) {
                Ok((written, deleted)) => {
                    report.artifacts_written = written;
                    report.artifacts_deleted = deleted;
                }
                Err(message) => report.fatal.push(message),
            }
        }
    }
    report.timings.write = start.elapsed();

    report
}
