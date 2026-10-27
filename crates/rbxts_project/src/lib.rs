//! A roblox-ts project as a tsrs program, loaded in-process: the tsconfig, the program, and type checking of the
//! project's own files; and the project setup that precedes transforming (`setup`): the Rojo resolver (`rojo`), the
//! path translator (`path_translator`) and roblox-ts's checks, shared with plugins as `ProjectServices`.
#![warn(missing_docs)]

pub mod graph;
pub mod nodepath;
pub mod packages;
pub mod path_translator;
pub mod read_check;
pub mod read_log;
pub mod rojo;
pub mod setup;
pub mod source_cache;
pub mod stamp;

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

/// What [`load`] loads: a tsconfig, from a working directory, with a checker count.
pub struct LoadOptions {
    /// Path to tsconfig.json, absolute or relative to `cwd`.
    pub tsconfig: String,
    /// The directory relative paths resolve against (the process's working directory, for `rbxtsc`).
    pub cwd: String,
    /// Number of checkers. 1 gives one checker over every file (symbol identity holds across files).
    pub checkers: usize,
    /// Parse and bind declaration-file members on first use. Only valid while no declaration file is type-checked.
    pub lazy_dts: bool,
    /// The program will be freed ([`Project::free`]) once a newer one replaces it (watch mode): its checkers allocate
    /// in regions of their own and put their diagnostics in others (tsrs's `checker_regions` and
    /// `checker_diagnostic_regions`), which the free gives back, and its files are bound before any checker exists
    /// (a checker binding them would bind them in its region, while the parsed files outlive the program).
    pub regions: bool,
}

/// A loaded project: its parsed tsconfig and its tsrs program, with the time each took.
pub struct Project {
    /// The parsed tsconfig, with the checker count override.
    pub config: P<ParsedCommandLine>,
    /// The program (leaked: tsrs programs live for the process).
    pub program: &'static Program,
    /// The number of checkers the program was made with.
    pub checkers: usize,
    /// Reading and parsing the tsconfig and the configs it extends.
    pub config_time: Duration,
    /// `new_program`: reading, parsing, binding and module resolution of every file.
    pub program_time: Duration,
    /// The tsconfig as the load was given it.
    tsconfig: String,
    /// The working directory the project was loaded from (TypeScript spelling).
    pub cwd: String,
    /// What the tsconfig parse and the program load read from the file system, when the load was asked to record it.
    pub reads: Option<Arc<ReadLog>>,
    /// The tsconfig has no `types`, and the program has TypeScript 5's automatic type directives (`"types": ["*"]`).
    pub automatic_types: bool,
    /// The tsconfig has no `rootDir`, and the program has TypeScript 5's common source directory as its `rootDir`
    /// ([`ts5_root_dir`]).
    pub ts5_root_dir: bool,
    /// [`Project::options`], made once.
    user_options: OnceLock<P<CompilerOptions>>,
    module_resolver: OnceLock<ModuleResolver>,
    /// The file system the program reads through (each program gets its own caching layer over it).
    fs: Arc<dyn FS>,
    /// The parsed files kept across loads, when the program was loaded with them ([`load_cached`]).
    source_cache: Option<Arc<SourceFileCache>>,
    /// The file-loader data the program shares with the programs [`Project::reuse`] made from it and the one it was
    /// made from: freed with the last of them ([`Project::free`]).
    lineage: Arc<Lineage>,
}

/// What the programs of one load and those reused from it share (`reuse_program` hands the loader's data on).
struct Lineage(tsrs_compiler::SharedProgramData);

/// A second owner of a project's program ([`Project::owner`]): a watch session keeps one for every program it has
/// not freed, so that the programs of a build that panics, whose `Project`s unwind with it, are freed rather than
/// leaked.
pub struct ProgramOwner {
    program: &'static Program,
    lineage: Arc<Lineage>,
}

impl ProgramOwner {
    /// Whether this owns `project`'s program.
    pub fn owns(&self, project: &Project) -> bool {
        std::ptr::eq(self.program, project.program)
    }

    /// [`Project::free`] through this owner.
    ///
    /// # Safety
    ///
    /// [`Project::free`]'s contract, and no `Project` of the program and no other owner of it is left (the loader's
    /// data goes with the last owner of the lineage).
    pub unsafe fn free(self) {
        // SAFETY: the caller's contract
        unsafe { free_program_and_lineage(self.program, self.lineage) };
    }
}

/// Frees `program`, and its lineage's data when no other program shares it.
///
/// # Safety
///
/// [`Project::free`]'s contract.
unsafe fn free_program_and_lineage(program: &'static Program, lineage: Arc<Lineage>) {
    // SAFETY: the caller's contract, which includes `free_program`'s.
    unsafe { tsrs_compiler::free_program(program) };
    // the last program of its lineage: nothing else shares the loader's data
    if let Ok(lineage) = Arc::try_unwrap(lineage) {
        // SAFETY: every other program of the lineage was freed (it held a reference to `lineage` until then), and
        // the caller's contract covers this one.
        unsafe { lineage.0.free() };
    }
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

/// TypeScript 5's automatic type directives (`getAutomaticTypeDirectiveNames`): without `types` in the tsconfig,
/// every package of the type roots is in the program (one whose package.json has `"typings": null` excepted, and
/// names starting with a dot). TypeScript 7 includes none then, so a project, or a referenced project, whose sources
/// import no package would not see the Roblox globals of `@rbxts/compiler-types`. TypeScript 7's `"types": ["*"]` is
/// TypeScript 5's rule, with TypeScript 5's wording where it differs (TS2688's "Entry point for implicit type library",
/// "Cannot find name 'process'" without "add node to the types field"): it goes into the parsed options when the
/// tsconfig has no `types`, and roblox-ts's own check of `types` sees the tsconfig's (`Project::automatic_types`).
/// Whether it did.
fn add_automatic_type_directives(parsed: &mut ParsedCommandLine) -> bool {
    let Some(options) = parsed.parsed_config.compiler_options.as_ref() else { return false };
    if options.types.is_some() {
        return false;
    }
    let mut options = CompilerOptions::clone(options);
    options.types = Some(vec!["*".to_owned()]);
    parsed.parsed_config.compiler_options = Some(P::new(options));
    true
}

/// TypeScript 5's output layout without `rootDir`: the common source directory of the files it emits
/// (`getCommonSourceDirectory`). TypeScript 7 lays the outputs out under the tsconfig's directory instead, and refuses
/// a project whose files have another common directory (`TS5011: The common source directory of 'tsconfig.json' is
/// './src'. The 'rootDir' setting must be explicitly set`), as a tsconfig with `rootDirs` but no `rootDir` meets
/// whenever its files share a folder below the tsconfig. So that such a project builds as with stock roblox-ts, that
/// directory goes into the program's options as `rootDir` when it is not the tsconfig's own: the outputs (the
/// declarations TypeScript emits included) are laid out as TypeScript 5 lays them out. roblox-ts sees the tsconfig's
/// options ([`Project::options`]).
///
/// `files` are the files TypeScript 5 would emit (its `sourceFileMayBeEmitted` ones, declarations excepted); `None`
/// when TypeScript 7 lays the outputs out by `rootDir` or the tsconfig anyway (tsrs's condition for TS5011: an output
/// directory, no `rootDir`, not composite). Returns the `rootDir` to set, if any.
fn ts5_root_dir(options: &CompilerOptions, user_root_dir: bool, files: &[String], cwd: &str) -> Option<String> {
    let laid_out_by_config = !options.no_emit.is_true()
        && !options.composite.is_true()
        && !user_root_dir
        && !options.config_file_path.is_empty()
        && (!options.out_dir.is_empty()
            || (options.get_emit_declarations() && !options.declaration_dir.is_empty())
            || !options.out_file.is_empty());
    if !laid_out_by_config {
        return None;
    }
    let case_sensitive = osvfs::fs().use_case_sensitive_file_names();
    let common = tsrs_tsoptions::outputpaths::get_computed_common_source_directory(files, cwd, case_sensitive);
    let config_dir = tspath::ensure_trailing_directory_separator(&tspath::get_directory_path(&options.config_file_path));
    (!common.is_empty()
        && tspath::get_canonical_file_name(&common, case_sensitive)
            != tspath::get_canonical_file_name(&config_dir, case_sensitive))
    .then(|| tspath::remove_trailing_directory_separator(&common).to_owned())
}

/// Sets the program's `rootDir` (`ts5_root_dir`), unless the tsconfig has one.
fn set_root_dir_if(parsed: &mut ParsedCommandLine, user_root_dir: bool, root_dir: Option<&str>) {
    let Some(root_dir) = root_dir.filter(|_| !user_root_dir) else { return };
    let Some(options) = parsed.parsed_config.compiler_options.as_ref() else { return };
    let mut options = CompilerOptions::clone(options);
    options.root_dir = root_dir.to_owned();
    parsed.parsed_config.compiler_options = Some(P::new(options));
}

/// The files of `program` TypeScript 5's `getCommonSourceDirectory` takes (as tsrs's TS5011 check takes them).
fn emitted_files(program: &Program) -> Vec<String> {
    program
        .source_files()
        .iter()
        .filter(|&&f| !f.is_declaration_file.get() && program.source_file_may_be_emitted(f, false))
        .map(|f| f.file_name().to_owned())
        .collect()
}

/// TypeScript 5's `baseUrl`, which TypeScript 7 removed (`TS5102: Option 'baseUrl' has been removed`; an import that
/// only resolves through it is `TS2307`). TypeScript 5 tries a non-relative module name against `paths` (whose
/// substitutions are relative to `baseUrl` then), and only when no pattern matched it, against `baseUrl` itself
/// (`tryLoadModuleUsingOptionalResolutionSettings`: a matched pattern whose substitutions all fail is the answer). The
/// same as `paths` alone: each substitution made absolute against `baseUrl`, and `"*": ["<baseUrl>/*"]` for the names
/// no pattern matches (a pattern matches by its longest prefix, so `*` only takes the rest; a `"*"` of the tsconfig's
/// own leaves no name to `baseUrl`); then `baseUrl` is cleared. Returns whether the tsconfig had one.
fn emulate_base_url(parsed: &mut ParsedCommandLine) -> bool {
    let Some(options) = parsed.parsed_config.compiler_options.as_ref() else { return false };
    if options.base_url.is_empty() {
        return false;
    }
    let mut options = CompilerOptions::clone(options);
    // absolute already (the parse resolves it against the tsconfig that sets it)
    let base_url = std::mem::take(&mut options.base_url);
    let under_base = |path: &str| tspath::normalize_path(&tspath::combine_paths(&base_url, &[path]));
    let mut paths = options.paths.take().unwrap_or_default();
    for substitutions in paths.values_mut() {
        let Some(substitutions) = substitutions else { continue };
        for substitution in substitutions.iter_mut() {
            *substitution = under_base(substitution);
        }
    }
    if !paths.contains_key("*") {
        paths.insert("*".to_owned(), Some(vec![under_base("*")]));
    }
    options.paths = Some(paths);
    parsed.parsed_config.compiler_options = Some(P::new(options));
    true
}

/// Parses the tsconfig and creates the program. Returns the config diagnostics when the config cannot be used.
/// `lazy_dts` switches a process-wide tsrs mode on for good: only for one-shot processes that never check a
/// declaration file.
pub fn load(opts: &LoadOptions) -> Result<Project, Vec<P<Diagnostic>>> {
    load_cached(opts, None, None)
}

/// Where a load records what it reads: the tsconfig parse (the configs, and the include glob's listings) and the
/// program (its files, module resolution) apart, as a watch rebuild can parse the tsconfig again on its own
/// ([`Project::reparse_config`]).
pub struct LoadLogs {
    /// The tsconfig parse's reads.
    pub config: Arc<ReadLog>,
    /// The program's reads.
    pub program: Arc<ReadLog>,
}

/// A file system that records what is read through it in `log`, when there is one.
fn recording_fs(log: Option<&Arc<ReadLog>>) -> Arc<dyn FS> {
    match log {
        Some(log) => Arc::new(read_log::RecordingFs { inner: bundled::wrap_fs(osvfs::fs()), log: Arc::clone(log) }),
        None => Arc::new(bundled::wrap_fs(osvfs::fs())),
    }
}

/// Parses the tsconfig as a load does: TypeScript 7's parse, with TypeScript 5's automatic type directives
/// ([`add_automatic_type_directives`], whether it added them) and `baseUrl` ([`emulate_base_url`]). On the compiler's
/// worker pool, as tsrs's CLI does: the include glob's parallel listing runs on it.
fn parse_config(
    tsconfig: &str,
    checkers: usize,
    host: &'static ConfigHost,
) -> Result<(ParsedCommandLine, bool), Vec<P<Diagnostic>>> {
    // Only the checker count: `single_threaded` would also make reading, parsing and binding serial.
    let overrides = CompilerOptions { checkers: Some(checkers as i64), ..CompilerOptions::default() };
    let (parsed, errors) = tsrs_compiler::worker_pool()
        .install(|| get_parsed_command_line_of_config_file(tsconfig, Some(&overrides), None, host, None));
    let Some(mut parsed) = parsed.filter(|_| errors.is_empty()) else {
        return Err(errors);
    };
    let automatic_types = add_automatic_type_directives(&mut parsed);
    emulate_base_url(&mut parsed);
    Ok((parsed, automatic_types))
}

/// [`load`] with parsed files kept in `cache` across loads (watch mode): a file whose stamp ([`stamp`]) and parse
/// options are unchanged is the same `SourceFile` in the new program. `logs`: where to record what the tsconfig
/// parse and the program load read (filled even when the load fails).
pub fn load_cached(
    opts: &LoadOptions,
    cache: Option<&Arc<SourceFileCache>>,
    logs: Option<&LoadLogs>,
) -> Result<Project, Vec<P<Diagnostic>>> {
    let cwd = tspath::normalize_path(&opts.cwd);
    let reads = logs.map(|logs| Arc::clone(&logs.program));
    let fs = recording_fs(reads.as_ref());
    let config_fs = recording_fs(logs.map(|logs| &logs.config));
    let host: &'static ConfigHost = Box::leak(Box::new(ConfigHost { fs: config_fs, cwd: cwd.clone() }));

    let start = Instant::now();
    let parse = || parse_config(&opts.tsconfig, opts.checkers, host);
    let (mut parsed, automatic_types) = parse()?;
    let config_time = start.elapsed();
    let user_root_dir = parsed.compiler_options().is_some_and(|o| !o.root_dir.is_empty());
    // TypeScript 5's common source directory, from the root files: the program's files are known once it is made,
    // and a file outside the root files' directory that the program reaches (rare) makes it again, below
    let roots: Vec<String> =
        parsed.file_names().iter().filter(|f| !tspath::is_declaration_file_name(f)).cloned().collect();
    let mut root_dir = parsed.compiler_options().and_then(|o| ts5_root_dir(&o, user_root_dir, &roots, &cwd));
    set_root_dir_if(&mut parsed, user_root_dir, root_dir.as_deref());
    let mut config = P::new(parsed);

    if opts.lazy_dts && tsrs_compiler::lazy_dts_allowed() {
        tsrs_compiler::enable_lazy_dts();
    }
    let start = Instant::now();
    let make = |config: P<ParsedCommandLine>| {
        let mut program_options = ProgramOptions::new(config, new_host(&cwd, &fs, cache, reads.as_ref()));
        program_options.checker_regions = opts.regions;
        program_options.checker_diagnostic_regions = opts.regions;
        new_program(program_options)
    };
    let mut program = make(config);
    if let Some(options) = config.compiler_options() {
        let wanted = ts5_root_dir(&options, user_root_dir, &emitted_files(program), &cwd);
        if wanted != root_dir {
            let (mut parsed, _) = parse()?;
            set_root_dir_if(&mut parsed, user_root_dir, wanted.as_deref());
            config = P::new(parsed);
            let first = program;
            program = make(config);
            // SAFETY: no checker of the first program was made, and nothing refers to it or its loader data
            unsafe {
                let lineage = Lineage(tsrs_compiler::shared_program_data(first));
                tsrs_compiler::free_program(first);
                lineage.0.free();
            }
            root_dir = wanted;
        }
    }
    if opts.regions {
        program.bind_source_files();
    }
    let program_time = start.elapsed();

    Ok(Project {
        config,
        program,
        checkers: opts.checkers,
        config_time,
        program_time,
        tsconfig: opts.tsconfig.clone(),
        cwd,
        reads,
        automatic_types,
        ts5_root_dir: root_dir.is_some(),
        user_options: OnceLock::new(),
        module_resolver: OnceLock::new(),
        lineage: Arc::new(Lineage(tsrs_compiler::shared_program_data(program))),
        fs,
        source_cache: cache.cloned(),
    })
}

/// A compiler host for one program: a fresh caching layer over the file system (directory listings and reads are
/// cached for the program's lifetime), serving parsed files from `cache` when there is one.
fn new_host(
    cwd: &str,
    fs: &Arc<dyn FS>,
    cache: Option<&Arc<SourceFileCache>>,
    reads: Option<&Arc<ReadLog>>,
) -> Arc<dyn tsrs_compiler::CompilerHost> {
    let host = new_cached_fs_compiler_host(cwd, Arc::clone(fs), &bundled::default_lib_path(), None, None, None);
    match cache {
        Some(cache) => Arc::new(source_cache::CachingHost { inner: host, cache: Arc::clone(cache), reads: reads.cloned() }),
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

    /// Whether a file could be included below the directory `dir` (any spelling): it is in or below a directory the
    /// `include` globs search (recursively, or that very directory), or above one, or above a file `files` lists.
    /// TypeScript's `possiblyMatchesDirectoryName`, plus the ancestors a directory created on the way passes through.
    pub fn possibly_includes_below(&self, dir: &str) -> bool {
        let config = &*self.0;
        let cwd = config.get_current_directory();
        let case_sensitive = config.use_case_sensitive_file_names();
        let dir_path = tspath::to_path(&tspath::normalize_slashes(dir), cwd, case_sensitive);
        if config.possibly_matches_directory_name(&dir_path) {
            return true;
        }
        let dir_key = dir_path.to_string();
        let below = |path: &str| {
            let key = tspath::to_path(path, cwd, case_sensitive).to_string();
            key.len() > dir_key.len() && key.starts_with(&dir_key) && key.as_bytes()[dir_key.len()] == b'/'
        };
        config.wildcard_directories().is_some_and(|dirs| dirs.iter().any(|(wildcard, _)| below(wildcard)))
            || config.literal_file_names().iter().any(|file| below(file))
    }

    /// The tsconfig's root files as its `files`, `include` and `exclude` find them on disk now (TypeScript's
    /// `getFileNamesFromConfigSpecs`, a new walk of the include globs), as path keys ([`IncludeMatcher::key`]).
    pub fn current_root_files(&self) -> std::collections::HashSet<String> {
        let reloaded = self.0.reload_file_names_of_parsed_command_line(&bundled::wrap_fs(osvfs::fs()));
        reloaded.file_names_by_path().keys().map(ToString::to_string).collect()
    }

    /// A file's path key, as [`IncludeMatcher::current_root_files`] lists it.
    pub fn key(&self, file_name: &str) -> String {
        let config = &*self.0;
        tspath::to_path(
            &tspath::normalize_slashes(file_name),
            config.get_current_directory(),
            config.use_case_sensitive_file_names(),
        )
        .to_string()
    }
}

impl Project {
    /// Frees the program with its checkers (their regions, with everything they allocated: types, symbols they made,
    /// diagnostics, and whatever was allocated while one was held, such as a transform's nodes and diagnostics) and its
    /// file-loader data. The parsed files, the tsconfig and the config parse stay (they are shared with later
    /// programs). A program loaded without [`LoadOptions::regions`] gives back only its own structures.
    ///
    /// # Safety
    ///
    /// Nothing uses the program or anything made while one of its checkers was held any more: no checker of it is
    /// held, and no diagnostic, type, symbol, signature or node made by its checkers or under them is read again (the
    /// caller copies the diagnostics it keeps: `DiagnosticExt::copy_deep`). No other program shares its data (a program
    /// from `load` never does).
    pub unsafe fn free(self) {
        // SAFETY: the caller's contract
        unsafe { free_program_and_lineage(self.program, self.lineage) };
    }

    /// Parses the tsconfig again, recording what it reads in `log`, and tells whether the parse finds the same files
    /// with the same automatic type directives. A watch rebuild asks when the parse's directory listings read
    /// differently (an output folder made at the tsconfig's directory, a file added that the include globs do not
    /// take): when the other answers are the same too ([`ReadLog::same_but_listings`]), the parse is this one, as
    /// the listings only decide the files.
    pub fn reparse_config(&self, log: &Arc<ReadLog>) -> bool {
        // leaked, as a load's is (the parse wants a host for the process): the caller empties the log it replaces
        let host: &'static ConfigHost =
            Box::leak(Box::new(ConfigHost { fs: recording_fs(Some(log)), cwd: self.cwd.clone() }));
        let Ok((parsed, automatic_types)) = parse_config(&self.tsconfig, self.checkers, host) else { return false };
        automatic_types == self.automatic_types && parsed.file_names() == self.config.file_names()
    }

    /// A second owner of the program ([`ProgramOwner`]). While it lives, freeing the program through its
    /// `Project` keeps the lineage's data: drop it (or free through it) first.
    pub fn owner(&self) -> ProgramOwner {
        ProgramOwner { program: self.program, lineage: Arc::clone(&self.lineage) }
    }

    /// A program for edited files, made from this one with tsrs's `reuse_program` (the path tsrs's own watcher takes):
    /// each file of `changed` (file names) is parsed again and replaces its old version; every other file, the module
    /// resolution and the options are this program's. With no file changed, a program over the same files, with
    /// checkers of its own. `None` when tsrs cannot reuse the program for a file (its imports, references or module
    /// augmentations changed, or it is not in the program): load the project again.
    ///
    /// The caller has made sure that nothing the load read reads differently now ([`ReadLog::recheck`]): the module
    /// resolution kept is then the one a new load would make. The parsed files go to this project's source cache.
    pub fn reuse(&self, changed: &[String]) -> Option<Project> {
        let host = new_host(&self.cwd, &self.fs, self.source_cache.as_ref(), self.reads.as_ref());
        let case_sensitive = self.fs.use_case_sensitive_file_names();
        let start = Instant::now();
        // with nothing changed, the program is made again over one of its own files, unchanged
        let unchanged = self.program.source_files().iter().find(|f| !f.get().file_name().starts_with("bundled:"));
        let names: Vec<String> = if changed.is_empty() {
            vec![unchanged?.get().file_name().to_owned()]
        } else {
            changed.to_vec()
        };
        let mut program = self.program;
        for file_name in &names {
            let path = tspath::to_path(file_name, &self.cwd, case_sensitive);
            let reused = program
                .get_source_file_by_path(&path)
                .and_then(|_| {
                    let (reused, _, ok) = program.reuse_program(&path, Arc::clone(&host), None, None);
                    reused.filter(|_| ok)
                });
            if !std::ptr::eq(program, self.program) {
                // SAFETY: a program made by an earlier turn of this loop, which nothing refers to: no checker of it was
                // made, and the one reused from it (if any) shares only the lineage's data, which stays.
                unsafe { tsrs_compiler::free_program(program) };
            }
            program = reused?;
        }
        // the edited files are bound before any checker exists (`LoadOptions::regions`)
        program.bind_source_files();
        Some(Project {
            config: self.config,
            program,
            checkers: self.checkers,
            config_time: Duration::ZERO,
            program_time: start.elapsed(),
            tsconfig: self.tsconfig.clone(),
            cwd: self.cwd.clone(),
            reads: self.reads.clone(),
            module_resolver: OnceLock::new(),
            fs: Arc::clone(&self.fs),
            source_cache: self.source_cache.clone(),
            lineage: Arc::clone(&self.lineage),
            automatic_types: self.automatic_types,
            ts5_root_dir: self.ts5_root_dir,
            user_options: OnceLock::new(),
        })
    }

    /// The compiler options as roblox-ts (and a plugin) sees them: the tsconfig's, without the `rootDir` rbxts-rs
    /// gives the program for TypeScript 5's output layout ([`ts5_root_dir`]). The program's own (`program.options()`)
    /// are what TypeScript sees.
    pub fn options(&self) -> P<CompilerOptions> {
        if !self.ts5_root_dir {
            return self.program.options();
        }
        *self.user_options.get_or_init(|| {
            let mut options = CompilerOptions::clone(&self.program.options());
            options.root_dir = String::new();
            P::new(options)
        })
    }

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

    /// What TypeScript's `getPreEmitDiagnostics` reports for `files` (roblox-ts's check): syntactic diagnostics, then
    /// bind and semantic diagnostics with `@ts-ignore` / `@ts-expect-error` / `@ts-nocheck` and `skipLibCheck`
    /// applied (tsrs's `get_semantic_diagnostics_with_checker`), in a declaration build the declaration diagnostics,
    /// then the global diagnostics; sorted and deduplicated. With one checker every file is checked on the calling thread; with more, one task per checker
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
        if program.options().get_emit_declarations() {
            // each takes its file's checker (none is held here)
            for (diags, &f) in per_file.iter_mut().zip(files) {
                diags.extend(program.get_declaration_diagnostics(ctx, Some(f)));
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
        self.module_resolver().resolve(specifier, from.get().file_name(), mode)
    }

    /// A resolver with the program's options and directory, for specifiers the program itself did not resolve: one
    /// per project load, made on first use, shared by every checker's transform.
    pub fn module_resolver(&self) -> &ModuleResolver {
        self.module_resolver.get_or_init(|| ModuleResolver::new(self.program.options(), &self.cwd))
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
        self.setup_into(self.reads.as_ref(), tsconfig_path, overrides)
    }

    /// [`Project::setup`], recording what it reads in `reads` (a log of its own, so that a watch rebuild can tell
    /// the setup's reads from the program's).
    pub fn setup_into(
        &self,
        reads: Option<&Arc<ReadLog>>,
        tsconfig_path: &str,
        overrides: &ProjectOptionLayer,
    ) -> Result<ProjectSetup, ProjectError> {
        read_log::recording(reads, || self.setup_recorded(reads.map(|r| &**r), tsconfig_path, overrides))
    }

    fn setup_recorded(
        &self,
        reads: Option<&ReadLog>,
        tsconfig_path: &str,
        overrides: &ProjectOptionLayer,
    ) -> Result<ProjectSetup, ProjectError> {
        let tsconfig_path = if nodepath::is_absolute(tsconfig_path) {
            tsconfig_path.to_owned()
        } else {
            nodepath::resolve(&[tsconfig_path])
        };
        // as TypeScript's solution builder spells the config path for roblox-ts (forward slashes, in messages)
        let rbxts = setup::read_project_options(&tspath::normalize_path(&tsconfig_path))?;
        let options = ProjectOptions::from_layers(&[&rbxts.options, overrides]);
        let mut data = setup::create_project_data_recorded(&tsconfig_path, options, reads)?;
        data.warnings.splice(0..0, rbxts.warnings);
        self.setup_with_data_recorded(reads, data)
    }

    /// [`Project::setup`] from project data made already: by [`graph::ProjectGraph`] for a project of a build with
    /// project references, whose options and Rojo file depend on the graph.
    pub fn setup_with_data(&self, data: ProjectData) -> Result<ProjectSetup, ProjectError> {
        self.setup_with_data_into(self.reads.as_ref(), data)
    }

    /// [`Project::setup_with_data`], recording what it reads in `reads` ([`Project::setup_into`]).
    pub fn setup_with_data_into(
        &self,
        reads: Option<&Arc<ReadLog>>,
        data: ProjectData,
    ) -> Result<ProjectSetup, ProjectError> {
        read_log::recording(reads, || self.setup_with_data_recorded(reads.map(|r| &**r), data))
    }

    fn setup_with_data_recorded(&self, reads: Option<&ReadLog>, data: ProjectData) -> Result<ProjectSetup, ProjectError> {
        let tsconfig_path = data.tsconfig_path.clone();
        let raw_options = self.options();
        // getParsedCommandLine: a composite project without rootDir gets the tsconfig's directory, before validation
        // and for everything after it (the build info path was already computed from the raw options)
        let mut options = (*raw_options).clone();
        if options.composite.is_true() && options.root_dir.is_empty() {
            options.root_dir = nodepath::dirname(&tsconfig_path).to_owned();
        }
        // roblox-ts checks the tsconfig's `types`, not the automatic type directives
        if self.automatic_types {
            options.types = None;
        }
        setup::validate_compiler_options(&options, &data.project_path)?;
        let case_sensitive = osvfs::fs().use_case_sensitive_file_names();
        let translator = setup::create_path_translator(&raw_options, self.config.file_names(), &data, case_sensitive);
        ProjectSetup::new_recorded(data, translator, &options, &self.input_files(), case_sensitive, reads)
    }
}
