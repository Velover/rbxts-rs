//! A roblox-ts project as a tsrs program, loaded in-process: the tsconfig, the program, and type checking of the
//! project's own files.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
    let cwd = tspath::normalize_path(&opts.cwd);
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
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
    let compiler_host =
        new_cached_fs_compiler_host(&cwd, fs, &bundled::default_lib_path(), None, None, None);
    let start = Instant::now();
    let program = new_program(ProgramOptions::new(config, compiler_host));
    let program_time = start.elapsed();

    Ok(Project { config, program, checkers: opts.checkers, config_time, program_time })
}

impl Project {
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
}
