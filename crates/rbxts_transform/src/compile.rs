//! The build pipeline (`Project/functions/compileFiles.ts`, DESIGN §4): load, check every project file, set up the
//! project, transform each file in program order, render, and write the outputs that changed.
//!
//! Any error stops the build before anything is written: checker and setup errors before the transform, transform
//! errors before rendering (roblox-ts's rule). [`CompileOptions::write_partial`] relaxes the second for parity
//! runs: files that transformed cleanly are written even when other files failed.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use rbxts_luau::{LateStrings, Stmt};
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
    pub transform: Duration,
    pub render: Duration,
    pub write: Duration,
}

/// One project file's outcome.
pub struct FileOutcome {
    pub file_name: String,
    /// Where the output goes (after `out_dir_override`).
    pub out_path: String,
    /// The transform's diagnostics (roblox-ts's and rbxts-rs's own).
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
    /// Setup warnings (Rojo config search, resolver warnings).
    pub warnings: Vec<String>,
    /// A failure that stopped the build, with its messages (roblox-ts's `ProjectError` / text diagnostics).
    pub fatal: Vec<String>,
    /// One per transformed file, in program order.
    pub files: Vec<FileOutcome>,
    /// Where the outputs go: `outDir`, or the override.
    pub out_dir: String,
    /// Files in the program, and project files transformed.
    pub program_files: usize,
    pub project_files: usize,
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

/// One file's transform: its Luau (unless it panicked) and what it reported.
struct FileTransform {
    ast: Option<Vec<Stmt>>,
    diagnostics: Vec<P<Diagnostic>>,
    unsupported: Vec<String>,
    internal_errors: Vec<String>,
}

fn transform_file(
    project: &Project,
    checker: &mut tsrs_checker::Checker,
    services: &ProjectServices,
    macro_manager: &MacroManager,
    multi: &mut MultiTransformState,
    file: P<SourceFile>,
) -> FileTransform {
    reset_temp_ids();
    let mut state = TransformState::new(project.program, checker, services, macro_manager, multi, file);
    let result = catch_unwind(AssertUnwindSafe(|| transform_source_file(&mut state, file)));
    let mut internal_errors = Vec::new();
    let ast = match result {
        Ok(ast) => Some(ast),
        Err(panic) => {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "panic".to_owned());
            state.add_diagnostic(internal_error(file.as_node(), &message));
            internal_errors.push(message);
            None
        }
    };
    let diagnostics = std::mem::take(&mut state.diagnostics.diagnostics);
    let unsupported = std::mem::take(&mut state.diagnostics.unsupported);
    FileTransform { ast, diagnostics, unsupported, internal_errors }
}

/// Builds a project: check → transform → render → write (DESIGN §4). Runs on the calling thread, which needs a
/// large stack ([`rbxts_project::STACK_SIZE`]).
pub fn compile_project(options: &CompileOptions) -> CompileReport {
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
    if report.diagnostics.iter().any(|d| d.category() == Category::Error) {
        return report;
    }

    // transform: one checker, files in program order
    let start = Instant::now();
    let mut asts: Vec<Option<Vec<Stmt>>> = Vec::with_capacity(files.len());
    {
        let mut checker = project.program.get_type_checker(&ctx);
        let macro_manager = match MacroManager::new(&mut checker) {
            Ok(manager) => manager,
            Err(error) => {
                report.fatal = error.messages;
                return report;
            }
        };
        let mut multi = MultiTransformState::new();
        for &file in &files {
            let FileTransform { ast, diagnostics, unsupported, internal_errors } =
                transform_file(&project, &mut checker, &services, &macro_manager, &mut multi, file);
            let file_name = file.get().file_name().to_owned();
            let out_path = output_path(&services, &file_name, options.out_dir_override.as_deref());
            report.files.push(FileOutcome {
                file_name,
                out_path,
                diagnostics,
                unsupported,
                internal_errors,
                luau: None,
                written: false,
            });
            asts.push(ast);
        }
    }
    report.timings.transform = start.elapsed();

    let any_errors = report.files.iter().any(|f| !f.is_clean());
    if any_errors && !options.write_partial {
        return report;
    }

    // render (after the transform, so later plugin late strings can be filled in: DESIGN §4 step 6)
    let start = Instant::now();
    let late_strings = LateStrings::new();
    for (outcome, ast) in report.files.iter_mut().zip(asts) {
        if let Some(ast) = ast
            && outcome.is_clean()
        {
            outcome.luau = Some(rbxts_luau::render(&ast, &late_strings));
        }
    }
    report.timings.render = start.elapsed();

    // write: only outputs that changed
    let start = Instant::now();
    if !options.no_write {
        for outcome in &mut report.files {
            let Some(source) = &outcome.luau else { continue };
            let unchanged = std::fs::read(&outcome.out_path).is_ok_and(|existing| existing == source.as_bytes());
            if unchanged {
                continue;
            }
            if let Some(dir) = std::path::Path::new(&outcome.out_path).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::write(&outcome.out_path, source) {
                Ok(()) => outcome.written = true,
                Err(e) => report.fatal.push(format!("could not write {}: {e}", outcome.out_path)),
            }
        }
    }
    report.timings.write = start.elapsed();

    report
}
