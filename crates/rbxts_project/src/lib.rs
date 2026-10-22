//! A roblox-ts project as a tsrs program, loaded in-process: the tsconfig, the program, and type checking of the
//! project's own files; and the project setup that precedes transforming (`setup`): the Rojo resolver (`rojo`), the
//! path translator (`path_translator`) and roblox-ts's checks, shared with plugins as `ProjectServices`.

pub mod nodepath;
pub mod packages;
pub mod path_translator;
pub mod read_log;
pub mod rojo;
pub mod setup;
pub mod source_cache;

pub use packages::{ModuleResolver, PackageInfo, PackageJsonCache, ResolvedModuleInfo};
pub use path_translator::PathTranslator;
pub use read_log::{ReadLog, Seen, text_hash};
pub use rojo::{FileRelation, NetworkType, Partition, RbxPath, RbxType, RelativeRbxPathPart, RojoError, RojoResolver};
pub use setup::{
    FileRojoInfo, InputFile, ProjectData, ProjectDiagnostic, ProjectError, ProjectOptionLayer, ProjectOptions,
    ProjectServices, ProjectSetup, ProjectType, Severity,
};
pub use source_cache::SourceFileCache;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tsrs_ast::{Diagnostic, SourceFile, is_json_source_file};
use tsrs_compiler::{
    Context, Program, ProgramOptions, new_cached_fs_compiler_host, new_program, sort_and_deduplicate_diagnostics,
};
use tsrs_core::{CompilerOptions, P, tspath};
use tsrs_tsoptions::{ParseConfigHost, ParsedCommandLine, get_parsed_command_line_of_config_file};
use tsrs_vfs::{FS, bundled, osvfs};

/// Stack size for any thread that loads, checks or transforms: checking recurses deeply (tsrs's CLI uses the same).
pub const STACK_SIZE: usize = 512 << 20;

pub struct LoadOptions {
    /// Path to tsconfig.json, absolute or relative to `cwd`.
    pub tsconfig: String,
    pub cwd: String,
    /// Number of checkers. 1 gives one checker over every file (symbol identity holds across files).
    pub checkers: usize,
    /// Parse and bind declaration-file members on first use. Only valid while no declaration file is type-checked.
    pub lazy_dts: bool,
}

pub struct Project {
    pub config: P<ParsedCommandLine>,
    pub program: &'static Program,
    pub checkers: usize,
    pub config_time: Duration,
    /// `new_program`: reading, parsing, binding and module resolution of every file.
    pub program_time: Duration,
    /// The working directory the project was loaded from (TypeScript spelling).
    pub cwd: String,
    /// What the tsconfig parse and the program load read from the file system, when the load was asked to record it.
    pub reads: Option<Arc<ReadLog>>,
    module_resolver: OnceLock<ModuleResolver>,
}

struct ConfigHost {
    fs: Arc<dyn FS>,
    cwd: String,
}

impl ParseConfigHost for ConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }

    fn get_current_directory(&self) -> &str {
        &self.cwd
    }
}

/// Parses the tsconfig and creates the program. Returns the config diagnostics when the config cannot be used.
/// `lazy_dts` switches a process-wide tsrs mode on for good: only for one-shot processes that never check a
/// declaration file.
pub fn load(opts: &LoadOptions) -> Result<Project, Vec<P<Diagnostic>>> {
    load_cached(opts, None, None)
}

/// [`load`] with parsed files kept in `cache` across loads (watch mode): a file whose modification time, size
/// and parse options are unchanged is the same `SourceFile` in the new program. `reads`: where to record what the
/// tsconfig parse and the program load read (filled even when the load fails).
pub fn load_cached(
    opts: &LoadOptions,
    cache: Option<&Arc<SourceFileCache>>,
    reads: Option<Arc<ReadLog>>,
) -> Result<Project, Vec<P<Diagnostic>>> {
    let cwd = tspath::normalize_path(&opts.cwd);
    let fs: Arc<dyn FS> = match &reads {
        Some(log) => Arc::new(read_log::RecordingFs { inner: bundled::wrap_fs(osvfs::fs()), log: Arc::clone(log) }),
        None => Arc::new(bundled::wrap_fs(osvfs::fs())),
    };
    let host: &'static ConfigHost = Box::leak(Box::new(ConfigHost { fs: Arc::clone(&fs), cwd: cwd.clone() }));

    // Only the checker count: `single_threaded` would also make reading, parsing and binding serial.
    let overrides = CompilerOptions { checkers: Some(opts.checkers as i64), ..CompilerOptions::default() };
    let start = Instant::now();
    // On the compiler's worker pool, as tsrs's CLI does: the include glob's parallel listing runs on it.
    let (parsed, errors) = tsrs_compiler::worker_pool()
        .install(|| get_parsed_command_line_of_config_file(&opts.tsconfig, Some(&overrides), None, host, None));
    let config_time = start.elapsed();
    let Some(parsed) = parsed.filter(|_| errors.is_empty()) else {
        return Err(errors);
    };
    let config = P::new(parsed);

    if opts.lazy_dts && tsrs_compiler::lazy_dts_allowed() {
        tsrs_compiler::enable_lazy_dts();
    }
    let compiler_host = new_host(&cwd, &fs, cache);
    let start = Instant::now();
    let program = new_program(ProgramOptions::new(config, compiler_host));
    let program_time = start.elapsed();

    Ok(Project {
        config,
        program,
        checkers: opts.checkers,
        config_time,
        program_time,
        cwd,
        reads,
        module_resolver: OnceLock::new(),
    })
}

/// A compiler host for one program: a fresh caching layer over the file system (directory listings and reads are
/// cached for the program's lifetime), serving parsed files from `cache` when there is one.
fn new_host(cwd: &str, fs: &Arc<dyn FS>, cache: Option<&Arc<SourceFileCache>>) -> Arc<dyn tsrs_compiler::CompilerHost> {
    let host = new_cached_fs_compiler_host(cwd, Arc::clone(fs), &bundled::default_lib_path(), None, None, None);
    match cache {
        Some(cache) => Arc::new(source_cache::CachingHost { inner: host, cache: Arc::clone(cache) }),
        None => host,
    }
}

/// Whether a file is, or would be once created, part of a project's root files: TypeScript's
/// `possiblyMatchesFileName` over the tsconfig's `files` and `include` globs (false positives possible, never a
/// false negative). A copyable handle on the parsed tsconfig, which lives as long as the process.
#[derive(Clone, Copy)]
pub struct IncludeMatcher(P<ParsedCommandLine>);

impl IncludeMatcher {
    /// `file_name` in any spelling.
    pub fn possibly_includes(&self, file_name: &str) -> bool {
        self.0.possibly_matches_file_name(&tspath::normalize_slashes(file_name))
    }
}

impl Project {
    /// What [`IncludeMatcher`] checks, for this project's tsconfig.
    pub fn include_matcher(&self) -> IncludeMatcher {
        IncludeMatcher(self.config)
    }

    /// The declaration outputs TypeScript would emit for a project file (`ts.getOutputFileNames` filtered to
    /// `.d.ts` and `.d.ts.map`, as roblox-ts keeps them): none unless declarations are emitted, and none for JSON
    /// or declaration files.
    pub fn declaration_outputs(&self, file_name: &str) -> Vec<String> {
        let options = self.program.options();
        if !options.get_emit_declarations()
            || tspath::is_declaration_file_name(file_name)
            || tspath::file_extension_is(file_name, tspath::EXTENSION_JSON)
        {
            return Vec::new();
        }
        let declaration =
            tsrs_tsoptions::outputpaths::get_output_declaration_file_name_worker(file_name, &options, &*self.config);
        if declaration.is_empty() {
            return Vec::new();
        }
        let mut outputs = vec![declaration.clone()];
        if options.get_are_declaration_maps_enabled() {
            outputs.push(format!("{declaration}.map"));
        }
        outputs
    }

    /// The files roblox-ts compiles: not declaration files, not JSON modules, not from external libraries
    /// (node_modules).
    pub fn project_files(&self) -> Vec<P<SourceFile>> {
        self.program
            .source_files()
            .iter()
            .copied()
            .filter(|&f| {
                !f.is_declaration_file()
                    && !is_json_source_file(f)
                    && !self.program.is_source_file_from_external_library(f)
            })
            .collect()
    }

    /// Diagnostics of the tsconfig, the options and the program as a whole.
    pub fn program_diagnostics(&self) -> Vec<P<Diagnostic>> {
        let mut diags = self.program.get_config_file_parsing_diagnostics();
        diags.extend(self.program.get_program_diagnostics());
        diags
    }

    /// What TypeScript's `getPreEmitDiagnostics` reports for `files` (roblox-ts's check; declaration diagnostics,
    /// which it adds when declaration emit is on, are not included yet): syntactic diagnostics, then
    /// bind and semantic diagnostics with `@ts-ignore` / `@ts-expect-error` / `@ts-nocheck` and `skipLibCheck`
    /// applied (tsrs's `get_semantic_diagnostics_with_checker`), then the global diagnostics; sorted and
    /// deduplicated. With one checker every file is checked on the calling thread; with more, one task per checker
    /// (as tsrs's type-check pass, without its work stealing). Unlike the tsrs CLI, semantic diagnostics are reported
    /// even when a file has syntax errors, as `getPreEmitDiagnostics` does.
    pub fn check_files(&self, ctx: &Context, files: &[P<SourceFile>]) -> Vec<P<Diagnostic>> {
        let program = self.program;
        let mut per_file: Vec<Vec<P<Diagnostic>>> =
            files.iter().map(|&f| program.get_syntactic_diagnostics(ctx, Some(f))).collect();
        if self.checkers == 1 {
            // Released before the global diagnostics below, which take the pool's checkers themselves.
            let mut checker = program.get_type_checker(ctx);
            for (diags, &f) in per_file.iter_mut().zip(files) {
                diags.extend(program.get_semantic_diagnostics_with_checker(ctx, &mut checker, f));
            }
        } else {
            let index: HashMap<P<SourceFile>, usize> = files.iter().enumerate().map(|(i, &f)| (f, i)).collect();
            let results = Mutex::new(Vec::with_capacity(files.len()));
            let ran = program.for_each_checker_group(files, |checker, _, f| {
                let diags = program.get_semantic_diagnostics_with_checker(ctx, checker, f);
                results.lock().unwrap().push((index[&f], diags));
            });
            assert!(ran, "the program has no checker pool of its own");
            for (i, diags) in results.into_inner().unwrap() {
                per_file[i].extend(diags);
            }
        }
        let mut diags: Vec<P<Diagnostic>> = per_file.into_iter().flatten().collect();
        diags.extend(program.get_global_diagnostics(ctx));
        sort_and_deduplicate_diagnostics(&diags)
    }

    /// The number of checkers the program really has (tsrs caps the requested count by the file count).
    pub fn checker_count(&self) -> usize {
        self.program.checker_count()
    }

    /// Where `specifier` resolves when imported from `from` (Flamework's `resolveModule`): the program's own
    /// resolution when the file imports it, else a resolver with the program's options.
    pub fn resolve_module(&self, specifier: &str, from: P<SourceFile>) -> Option<ResolvedModuleInfo> {
        let mode = self.program.get_default_resolution_mode_for_file(from);
        if let Some(resolved) = self.program.get_resolved_module(from, specifier, mode).filter(|r| r.is_resolved()) {
            return Some(ResolvedModuleInfo {
                file_name: resolved.resolved_file_name.to_owned(),
                is_external_library_import: resolved.is_external_library_import,
                package_name: (!resolved.package_id.name.is_empty()).then(|| resolved.package_id.name.to_owned()),
            });
        }
        self.module_resolver
            .get_or_init(|| ModuleResolver::new(self.program.options(), &self.cwd))
            .resolve(specifier, from.get().file_name(), mode)
    }

    /// Every program source file in program order, with what setup needs to know of it.
    pub fn input_files(&self) -> Vec<InputFile<'static>> {
        self.program
            .source_files()
            .iter()
            .map(|&f| InputFile {
                file_name: f.get().file_name(),
                is_declaration: f.is_declaration_file(),
                is_external_library: self.program.is_source_file_from_external_library(f),
            })
            .collect()
    }

    /// roblox-ts's setup for this project: the `rbxts` options under `overrides` (command-line options), the project
    /// data, the compiler-option checks, the path translator and the Rojo resolvers, then `compileFiles`'s checks.
    /// `tsconfig_path` is used as spelled when absolute (roblox-ts's CLI passes it through `path.resolve`), else
    /// resolved against the current directory; the spelling carries into the project's paths.
    pub fn setup(&self, tsconfig_path: &str, overrides: &ProjectOptionLayer) -> Result<ProjectSetup, ProjectError> {
        let tsconfig_path = if nodepath::is_absolute(tsconfig_path) {
            tsconfig_path.to_owned()
        } else {
            nodepath::resolve(&[tsconfig_path])
        };
        // as TypeScript's solution builder spells the config path for roblox-ts (forward slashes, in messages)
        let rbxts = setup::read_project_options(&tspath::normalize_path(&tsconfig_path))?;
        let options = ProjectOptions::from_layers(&[&rbxts.options, overrides]);
        let reads = self.reads.as_deref();
        let mut data = setup::create_project_data_recorded(&tsconfig_path, options, reads)?;
        data.warnings.splice(0..0, rbxts.warnings);
        let raw_options = self.program.options();
        // getParsedCommandLine: a composite project without rootDir gets the tsconfig's directory, before validation
        // and for everything after it (the build info path was already computed from the raw options)
        let mut options = (*raw_options).clone();
        if options.composite.is_true() && options.root_dir.is_empty() {
            options.root_dir = nodepath::dirname(&tsconfig_path).to_owned();
        }
        setup::validate_compiler_options(&options, &data.project_path)?;
        let case_sensitive = osvfs::fs().use_case_sensitive_file_names();
        let translator = setup::create_path_translator(&raw_options, self.config.file_names(), &data, case_sensitive);
        ProjectSetup::new_recorded(data, translator, &options, &self.input_files(), case_sensitive, reads)
    }
}
