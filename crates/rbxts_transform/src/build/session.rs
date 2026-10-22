//! [`BuildSession`]: the build lifecycle of DESIGN §14 over one project, for one build (`rbxtsc build`) or many
//! (watch mode, the parity harness).
//!
//! A build is `build(kind, changes)`:
//! 1. **Load the program** and the project setup, every build, as roblox-ts does: module resolution, the Rojo
//!    resolvers and the package mapping are redone (a package installed or upgraded under `node_modules`, which no
//!    watcher sees, is picked up). Unchanged files come from the session's [`SourceFileCache`] (the same
//!    `SourceFile` objects), so only edited files are parsed again. (tsrs's `reuse_program` would keep the old
//!    module resolution: an import that failed to resolve stays unresolved after its package is installed.)
//! 2. **Decide the files to transform** ([`super::graph`]): every project file in a clean build, after a change of
//!    the salt (compiler, options, configs, Rojo files, plugin list), after a plugin's `setup` re-ran, or in an
//!    incremental build with plugins (their file parts are not persisted); otherwise roblox-ts's closure with
//!    TypeScript's global rule and the augmentation edges, plus the project files whose output is missing.
//! 3. **Check** those files (the others are unchanged since a build without errors).
//! 4. **Plugins:** per plugin, `rebuild_start` on a watch rebuild with a kept `Shared` and no changed build input, or
//!    `setup` on a working copy of the committed state (`default_state()` for clean and incremental builds).
//!    `rebuild_start` runs before the check, as the files to check depend on its answer.
//! 5. **Transform** the files; 6. plugin **`finish`** with the parts of all current files (cached for the files
//!    not transformed); 7. **render**, and re-render kept files whose late strings changed value;
//! 8. **commit** (no errors only): Luau and assets, artifacts, deletions (stale outputs, artifacts of the last build
//!    not written by this one, removals), plugin state files, then the incremental state; 9. the working states
//!    and `Shared`s become the committed ones, with every file's facts, parts and late values.
//!
//! A build that does not commit leaves the committed state as it was: the next build diffs against the last
//! committed one, re-runs `setup` where this one made a new `Shared`, and gets the changes of both.

use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_luau::{LateStrings, Stmt};
use rbxts_plugin::erased::{ErasedPart, ErasedShared, ErasedState};
use rbxts_plugin::{BuildKind, ChangeSet, CheckerInfo, FileIndex, RebuildPlan, StateOrigin};
use rbxts_project::{LoadOptions, Project, ProjectDiagnostic, ProjectServices, SourceFileCache, nodepath};
use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_compiler::Context;
use tsrs_core::P;
use tsrs_diagnostics::Category;

use super::graph::{self, FileFacts, Version};
use super::outputs::{self, OutputInputs};
use super::state::{self, DiskState};
use crate::classes::macro_manager::MacroManager;
use crate::classes::multi_transform_state::MultiTransformState;
use crate::compile::{
    CompileOptions, CompileReport, FileOutcome, comment_directive_diagnostics, explain_failures, output_path,
    panic_message, quietly, record_timings, transform_file,
};
use crate::plugins::host::{HookTime, resolve_module_from};
use crate::plugins::lifecycle::{
    ActivePlugin, BuildInput, CommitQueue, Finish, PluginOptions, Setup, activate, commit_artifacts, commit_removals,
    commit_state_files, output_key, plain_late_key, run_finish, run_setup, write_if_changed,
};
use crate::plugins::spill::LoadCheck;
use crate::plugins::{HookKind, PluginHost};
use crate::project::copy_include::{IncludeCopy, prepare_copy_include};
use crate::shared::diagnostics::{internal_error, text_diagnostic};

/// A program and its project setup, kept for the next build.
struct Loaded {
    project: Project,
    services: ProjectServices,
    setup_diagnostics: Vec<ProjectDiagnostic>,
    setup_fatal: Option<String>,
    /// The tsconfig and the configs it extends.
    config_paths: Vec<String>,
    /// The salt without the plugin list: compiler, options, config and Rojo texts.
    base_salt: String,
    /// Text versions by file object, across every program of the session.
    versions: FxHashMap<P<SourceFile>, Version>,
}

/// A committed file: its facts and, for project files, what the plugins and the late-string re-render need.
struct CommittedFile {
    facts: FileFacts,
    /// Per active plugin: the file's part (project files only).
    parts: Vec<Option<ErasedPart>>,
    /// (plugin, namespaced key, committed value) of each late string the file's Luau uses.
    late: Vec<(usize, String, String)>,
    /// The file's Luau, kept in watch mode when it uses late strings (for the re-render).
    ast: Option<Vec<Stmt>>,
    /// Its load-check mark, kept with `ast` when a plugin emitted into it: a late string's new value can decide whether
    /// Luau accepts the re-rendered file (a folded comparison, a dead branch).
    load_check: Option<LoadCheck>,
}

/// A plugin's committed state, and the `Shared` made from it with the inputs its `setup` read.
struct PluginRecord {
    name: &'static str,
    state: ErasedState,
    shared: Option<ErasedShared>,
    inputs: Vec<BuildInput>,
}

/// What the last committed build left: in memory (watch), or read from the incremental state.
struct Committed {
    salt: Version,
    plugin_ids: Vec<(String, String)>,
    files: FxHashMap<String, CommittedFile>,
    plugins: Vec<PluginRecord>,
    artifacts: Vec<String>,
    /// The directories plugins asked to watch (`watch_dir`).
    watch_dirs: Vec<String>,
    /// Read from the incremental state: no parts, no plugin states.
    from_disk: bool,
}

/// One project's builds: the committed state, the parsed files and the last program, across builds.
pub struct BuildSession<'p> {
    pub options: CompileOptions,
    plugins: &'p PluginOptions<'p>,
    /// Keep what watch rebuilds need in memory (the Luau of files using late strings).
    watch: bool,
    cache: Arc<SourceFileCache>,
    last: Option<Loaded>,
    committed: Option<Committed>,
    /// Paths touched since the last committed build.
    pending: BTreeSet<String>,
    /// Watch rebuilds started (the generation of the next one is this plus one).
    rebuilds: u64,
    /// Plugin states for a clean build (the harness's seeded clean build).
    seed: Option<Vec<(&'static str, ErasedState)>>,
    /// The incremental state was looked at (only the first build reads it).
    disk_checked: bool,
    /// What the last load read (recorded in watch sessions only), kept when the load failed.
    reads: Option<Arc<rbxts_project::ReadLog>>,
}

/// What one build decided, for reports and events.
#[derive(Clone, Debug, Default)]
pub struct BuildSummary {
    pub kind: Option<BuildKind>,
    /// Every project file was transformed.
    pub all: bool,
    /// Files parsed for this build (source cache misses).
    pub parsed: usize,
    /// Project files re-rendered because their late strings changed value.
    pub rerendered: Vec<String>,
    /// Outputs written (Luau, assets, artifacts, state files) and deleted.
    pub written: Vec<String>,
    pub deleted: Vec<String>,
    /// The changes since the last committed build.
    pub changes: ChangeSet,
}

impl<'p> BuildSession<'p> {
    /// A session over one project. `watch` keeps the Luau of files with late strings for later re-renders.
    pub fn new(options: CompileOptions, plugins: &'p PluginOptions<'p>, watch: bool) -> BuildSession<'p> {
        BuildSession {
            options,
            plugins,
            watch,
            cache: Arc::new(SourceFileCache::new()),
            last: None,
            committed: None,
            pending: BTreeSet::new(),
            rebuilds: 0,
            seed: None,
            disk_checked: false,
            reads: None,
        }
    }

    /// Starts the first (clean) build from these plugin states instead of `default_state()`: the harness's clean
    /// build seeded with a watcher's persistent state.
    pub fn with_seed(mut self, states: Vec<(&'static str, ErasedState)>) -> BuildSession<'p> {
        self.seed = Some(states);
        self.disk_checked = true;
        self
    }

    /// The committed plugin states, by plugin name (to seed a clean build with).
    pub fn committed_states(&self) -> Vec<(&'static str, ErasedState)> {
        self.committed.as_ref().map_or_else(Vec::new, |c| c.plugins.iter().map(|p| (p.name, p.state.clone())).collect())
    }

    /// A build has committed in this session (or the incremental state was read).
    pub fn has_committed(&self) -> bool {
        self.committed.is_some()
    }

    /// Forgets the committed state: the next build is a clean one.
    pub fn reset(&mut self) {
        self.committed = None;
        self.last = None;
        self.pending.clear();
        self.disk_checked = true;
    }

    /// The tsconfig and `extends` files, the Rojo project files and the plugin build inputs of the last loaded
    /// build: what a watcher watches besides the sources.
    pub fn config_inputs(&self) -> Vec<String> {
        let mut inputs = Vec::new();
        if let Some(last) = &self.last {
            inputs.extend(last.config_paths.iter().cloned());
            inputs.extend(last.services.rojo_resolver.config_files().iter().cloned());
        }
        if let Some(committed) = &self.committed {
            for plugin in &committed.plugins {
                for input in &plugin.inputs {
                    if let BuildInput::File { path, .. } = input {
                        inputs.push(path.clone());
                    }
                }
            }
        }
        inputs
    }

    /// The module resolutions plugins asked for in `setup` (`SetupCx::resolve_module`): (specifier, from file,
    /// resolved file). They are build inputs: a watcher watches the package directories they depend on, and a
    /// changed result runs `setup` again.
    pub fn plugin_resolutions(&self) -> Vec<(String, String, Option<String>)> {
        let mut out = Vec::new();
        if let Some(committed) = &self.committed {
            for plugin in &committed.plugins {
                for input in &plugin.inputs {
                    if let BuildInput::Resolution { specifier, from, resolved, .. } = input {
                        out.push((specifier.clone(), from.clone(), resolved.clone()));
                    }
                }
            }
        }
        out
    }

    /// The bare module specifiers the last loaded program could not resolve (packages not installed yet), for a
    /// watcher to watch where they would appear.
    pub fn unresolved_packages(&self) -> Vec<String> {
        let Some(last) = &self.last else { return Vec::new() };
        let mut names: Vec<String> = last
            .project
            .program
            .get_resolved_modules()
            .values()
            .flat_map(|cache| cache.iter())
            .filter(|(key, resolved)| {
                !resolved.is_resolved() && !key.name.starts_with('.') && !key.name.starts_with('/')
            })
            .map(|(key, _)| key.name.to_owned())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// What the last program load read from the file system (the tsconfig parse and the program: files, directories
    /// and the paths module resolution probed that did not exist), for a watcher; also when that load failed (a
    /// tsconfig extending a package not installed yet).
    pub fn read_log(&self) -> Option<Arc<rbxts_project::ReadLog>> {
        self.reads.clone()
    }

    /// The last program's files on disk, each with the modification time and size it had when parsed (a watcher
    /// compares them with the disk for directories it starts watching).
    pub fn loaded_stamps(&self) -> Vec<(String, Option<std::time::SystemTime>, i64)> {
        let Some(last) = &self.last else { return Vec::new() };
        last.project
            .program
            .source_files()
            .iter()
            .filter_map(|f| {
                let file = f.get();
                let (time, size) = self.cache.stamp(file.path())?;
                Some((file.file_name().to_owned(), time, size))
            })
            .collect()
    }

    /// The directories plugins asked the last committed build to watch (`FinishCx::watch_dir`).
    pub fn plugin_watch_dirs(&self) -> Vec<String> {
        self.committed.as_ref().map_or_else(Vec::new, |c| c.watch_dirs.clone())
    }

    /// The last loaded project's services, program file names and root directories (for a watcher's directory
    /// set).
    pub fn watch_roots(&self) -> Option<(&ProjectServices, Vec<String>, Vec<String>)> {
        let last = self.last.as_ref()?;
        let files = last.project.program.source_files().iter().map(|f| f.get().file_name().to_owned()).collect();
        let options = last.project.program.options();
        let mut roots = rbxts_project::setup::get_root_dirs(&options);
        if roots.is_empty() {
            roots.push(last.services.data.project_path.clone());
        }
        Some((&last.services, files, roots.iter().map(|r| nodepath::resolve(&[r])).collect()))
    }

    /// One build. `touched`: the paths a watcher saw change since the last build (empty for the first build).
    pub fn build(&mut self, touched: &[String]) -> (CompileReport, BuildSummary) {
        let committed = self.committed.take();
        let last = self.last.take();
        let (report, summary, committed, last) = self.run(touched, committed, last);
        self.committed = committed;
        self.last = last;
        (report, summary)
    }

    fn tsconfig_path(&self) -> String {
        if nodepath::is_absolute(&self.options.tsconfig) {
            self.options.tsconfig.clone()
        } else {
            nodepath::join(&[&self.options.cwd, &self.options.tsconfig])
        }
    }

    /// Loads the program and the project setup.
    fn load(&mut self, report: &mut CompileReport, versions: FxHashMap<P<SourceFile>, Version>) -> Option<Loaded> {
        self.reads = self.watch.then(|| Arc::new(rbxts_project::ReadLog::default()));
        let project = match rbxts_project::load_cached(
            &LoadOptions {
                tsconfig: self.options.tsconfig.clone(),
                cwd: self.options.cwd.clone(),
                checkers: 1,
                lazy_dts: false,
            },
            Some(&self.cache),
            self.reads.clone(),
        ) {
            Ok(project) => project,
            Err(diagnostics) => {
                report.diagnostics = diagnostics;
                report.fatal.push("the tsconfig could not be used".to_owned());
                return None;
            }
        };
        report.timings.config = project.config_time;
        report.timings.program = project.program_time;
        let start = Instant::now();
        let tsconfig_path = self.tsconfig_path();
        let setup = match project.setup(&tsconfig_path, &self.options.project_options) {
            Ok(setup) => setup,
            Err(error) => {
                report.fatal = error.messages;
                report.timings.setup = start.elapsed();
                return None;
            }
        };
        report.timings.setup = start.elapsed();
        let mut config_paths = vec![nodepath::normalize(&tsconfig_path)];
        config_paths.extend(project.config.extended_source_files().iter().map(|p| nodepath::normalize(p)));
        let base_salt = self.base_salt(&setup.services, &config_paths);
        Some(Loaded {
            project,
            services: setup.services,
            setup_diagnostics: setup.diagnostics,
            setup_fatal: setup.fatal,
            config_paths,
            base_salt,
            versions,
        })
    }

    /// The salt's text before the plugin list: what roblox-ts salts every file version with (compiler version,
    /// options, Rojo texts), with the config texts for the compiler options.
    fn base_salt(&self, services: &ProjectServices, config_paths: &[String]) -> String {
        let mut salt = String::new();
        salt.push_str(&state::compiler_identity());
        salt.push('\n');
        salt.push_str(&format!("{:?}\n{:?}\n", services.data.options, self.options.out_dir_override));
        salt.push_str(&format!("{:?}\n", services.project_type));
        // createNodeModulesPathMapping reads every type-root package's package.json: a changed main or types entry
        // changes the requires of the files importing it, with no change to any program file
        let mut mapping: Vec<_> = services.node_modules_path_mapping.iter().collect();
        mapping.sort();
        salt.push_str(&format!("{mapping:?}\n"));
        let mut texts = |paths: &[String]| {
            for path in paths {
                salt.push_str(path);
                salt.push('\0');
                let version = std::fs::read(path).map(|b| Version::of(&String::from_utf8_lossy(&b)));
                salt.push_str(&version.map_or_else(|_| "-".to_owned(), Version::to_hex));
                salt.push('\n');
            }
        };
        texts(config_paths);
        texts(services.rojo_resolver.config_files());
        for resolver in &services.pkg_rojo_resolvers {
            texts(resolver.config_files());
        }
        salt
    }

    #[allow(clippy::type_complexity)]
    fn run(
        &mut self,
        touched: &[String],
        mut committed: Option<Committed>,
        last: Option<Loaded>,
    ) -> (CompileReport, BuildSummary, Option<Committed>, Option<Loaded>) {
        let mut report = CompileReport::default();
        let mut summary = BuildSummary::default();
        self.pending.extend(touched.iter().cloned());
        let parses_before = self.cache.parses();

        // 1. load the program, over the source cache
        let loaded = self.load(&mut report, last.map(|l| l.versions).unwrap_or_default());
        let Some(loaded) = loaded else {
            summary.parsed = self.cache.parses() - parses_before;
            return (report, summary, committed, None);
        };
        summary.parsed = self.cache.parses() - parses_before;
        let mut loaded = loaded;
        let project = &loaded.project;
        let services = &loaded.services;
        report.program_files = project.program.source_files().len();
        report.warnings = services.warnings.clone();
        for d in &loaded.setup_diagnostics {
            let category =
                if d.severity == rbxts_project::Severity::Error { Category::Error } else { Category::Warning };
            report.diagnostics.push(text_diagnostic(&d.message, category));
        }
        if let Some(fatal) = &loaded.setup_fatal {
            report.fatal.push(fatal.clone());
            return (report, summary, committed, Some(loaded));
        }
        if project.program.options().assume_changes_only_affect_direct_dependencies.is_true() {
            report.fatal.push(
                "assumeChangesOnlyAffectDirectDependencies is not supported: it leaves the outputs of indirect \
                 importers stale (rbxts-rs re-transforms every file that reaches a changed file)"
                    .to_owned(),
            );
            return (report, summary, committed, Some(loaded));
        }
        report.out_dir =
            self.options.out_dir_override.clone().unwrap_or_else(|| services.path_translator.out_dir.clone());
        let tsconfig_path = self.tsconfig_path();
        let active = match activate(self.plugins, &tsconfig_path) {
            Ok(active) => active,
            Err(errors) => {
                report.fatal = errors;
                return (report, summary, committed, Some(loaded));
            }
        };
        report.plugins = active.iter().map(|p| p.info.name).collect();
        let plugin_ids: Vec<(String, String)> =
            active.iter().map(|p| (p.info.name.to_owned(), p.info.version.to_owned())).collect();
        let salt = {
            let mut text = loaded.base_salt.clone();
            for (plugin, (name, version)) in active.iter().zip(&plugin_ids) {
                text.push_str(&format!("plugin {name} {version} {:?}\n", plugin.entry));
            }
            Version::of(&text)
        };

        // the incremental state, for the first build of the session
        let state_path = self.state_path(services);
        if committed.is_none() && !self.disk_checked {
            self.disk_checked = true;
            if let Some(path) = &state_path
                && let Some(disk) = DiskState::read(path)
                && disk.compiler == state::compiler_identity()
                && disk.plugins == plugin_ids
            {
                committed = Some(Committed {
                    salt: disk.salt,
                    plugin_ids: disk.plugins,
                    files: disk
                        .files
                        .into_iter()
                        .map(|(k, facts)| {
                            (
                                k,
                                CommittedFile {
                                    facts,
                                    parts: Vec::new(),
                                    late: Vec::new(),
                                    ast: None,
                                    load_check: None,
                                },
                            )
                        })
                        .collect(),
                    plugins: Vec::new(),
                    artifacts: disk.artifacts,
                    watch_dirs: Vec::new(),
                    from_disk: true,
                });
            }
        }
        let kind = match &committed {
            None => BuildKind::Clean,
            Some(c) if c.from_disk => BuildKind::Incremental,
            Some(_) => {
                self.rebuilds += 1;
                BuildKind::WatchRebuild { generation: self.rebuilds }
            }
        };
        summary.kind = Some(kind);

        // 2. the files to transform
        let invalidate_start = Instant::now();
        // the outputs on disk before anything is written: the missing-output check and the clean
        let listing = outputs::list(
            services,
            &outputs::output_roots(services, &project.program.options(), self.options.out_dir_override.as_deref()),
            self.options.out_dir_override.as_deref(),
        );
        let facts = graph::program_facts(project.program, &mut loaded.versions);
        let project = &loaded.project;
        let services = &loaded.services;
        let mut all = match &committed {
            None => true,
            Some(c) => c.salt != salt || c.plugin_ids != plugin_ids || (c.from_disk && !active.is_empty()),
        };
        let mut changes = ChangeSet::default();
        let mut affected = FxHashSet::default();
        if let Some(c) = &committed {
            let invalidation = graph::invalidate(&c.files, |f| &f.facts, &facts);
            all |= invalidation.all;
            affected = invalidation.affected;
            changes.changed = invalidation.changed;
            changes.added = invalidation.added;
            changes.deleted = invalidation.deleted;
        }
        let program_keys: FxHashSet<String> = facts.iter().map(|(_, f)| path_key_of(services, &f.file_name)).collect();
        let config_keys: FxHashSet<String> = self.config_inputs_of(&loaded, committed.as_ref());
        for path in &self.pending {
            let key = path_key_of(services, path);
            if config_keys.contains(&key) {
                changes.config_inputs.push(path.clone());
            } else if !program_keys.contains(&key) {
                changes.non_ts.push(path.clone());
            }
        }

        // plugins: keep the Shared (rebuild_start) or set up again
        let resolver = OnceCell::new();
        let mut plans: Vec<Option<(ErasedShared, ErasedState, Vec<BuildInput>)>> = Vec::with_capacity(active.len());
        for plugin in &active {
            let record = committed.as_ref().and_then(|c| c.plugins.iter().find(|r| r.name == plugin.info.name));
            // a changed salt (options, configs, Rojo files, the plugin's tsconfig entry) is a changed build input
            let salt_changed = committed.as_ref().is_none_or(|c| c.salt != salt);
            let kept = match (kind, record) {
                (BuildKind::WatchRebuild { .. }, Some(record)) if !salt_changed => {
                    record.shared.as_ref().and_then(|shared| {
                        let changed = record.inputs.iter().any(|i| input_changed(i, project.program, &resolver));
                        if changed {
                            return None;
                        }
                        let plan = catch_unwind(AssertUnwindSafe(|| {
                            plugin.plugin.rebuild_start(shared, &record.state, &changes)
                        }));
                        match plan {
                            Ok(RebuildPlan::KeepShared) => {
                                Some((Arc::clone(shared), record.state.clone(), record.inputs.clone()))
                            }
                            Ok(RebuildPlan::RerunSetup) => None,
                            Err(panic) => {
                                report.fatal.push(format!(
                                    "plugin `{}` panicked in rebuild_start: {}",
                                    plugin.info.name,
                                    panic_message(&*panic)
                                ));
                                None
                            }
                        }
                    })
                }
                _ => None,
            };
            if kept.is_none() && matches!(kind, BuildKind::WatchRebuild { .. }) {
                all = true;
            }
            plans.push(kept);
        }
        if !report.fatal.is_empty() {
            return (report, summary, committed, Some(loaded));
        }
        summary.all = all;

        let files = project.project_files();
        report.project_files = files.len();
        // the output set and its errors before anything is compiled or written, as roblox-ts's getProjectOutputs
        // runs before compileFiles: two inputs emitting to one output, or an output root holding sources, write nothing
        let mut project_outputs = match self.project_outputs(&loaded, &files) {
            Ok(outputs) => outputs,
            Err(message) => {
                report.fatal.push(message);
                return (report, summary, committed, Some(loaded));
            }
        };
        let to_transform: Vec<P<SourceFile>> = if all {
            files.clone()
        } else {
            let committed = committed.as_ref().expect("a committed build");
            files
                .iter()
                .copied()
                .filter(|&file| {
                    let key = file.path().to_string();
                    if affected.contains(&key) {
                        return true;
                    }
                    // a file with no cached part for every plugin (it never transformed with them)
                    let cached = committed.files.get(&key);
                    if cached.is_none_or(|c| c.parts.len() != active.len() || c.parts.iter().any(Option::is_none)) {
                        return true;
                    }
                    // roblox-ts: any source whose output is missing
                    let out = output_path(services, file.get().file_name(), self.options.out_dir_override.as_deref());
                    !listing.contains(&path_key_of(services, &out))
                })
                .collect()
        };

        report.timings.invalidate = invalidate_start.elapsed();

        // 3. check the files to transform
        let start = Instant::now();
        let ctx = Context::background();
        report.diagnostics.extend(project.program_diagnostics());
        report.diagnostics.extend(project.check_files(&ctx, &to_transform));
        if !services.data.options.allow_comment_directives {
            for &file in &to_transform {
                report.diagnostics.extend(comment_directive_diagnostics(file));
            }
        }
        report.timings.check = start.elapsed();
        let check_failed = report.diagnostics.iter().any(|d| d.category() == Category::Error);

        // 4. plugin setup, also when the check failed (explain_checker_diagnostic needs the instances)
        let start = Instant::now();
        let mut run = None;
        if !active.is_empty() {
            let origin =
                if matches!(kind, BuildKind::WatchRebuild { .. }) { StateOrigin::Memory } else { StateOrigin::Fresh };
            let mut shared = Vec::with_capacity(active.len());
            let mut states = Vec::with_capacity(active.len());
            let mut inputs = Vec::with_capacity(active.len());
            let mut times = vec![[HookTime::default(); HookKind::ALL.len()]; active.len()];
            let change_set = (kind != BuildKind::Clean).then_some(&changes);
            for (index, (plugin, plan)) in active.iter().zip(plans).enumerate() {
                if let Some((kept, state, kept_inputs)) = plan {
                    shared.push(kept);
                    states.push(state);
                    inputs.push(kept_inputs);
                    continue;
                }
                let state = self.initial_state(plugin, kind, committed.as_ref());
                let mut backend = Setup {
                    program: project.program,
                    services,
                    plugin: plugin.info.name,
                    entry: plugin.entry.as_ref(),
                    resolver: &resolver,
                    inputs: Default::default(),
                    messages: &mut report.plugin_messages,
                    warnings: &mut report.warnings,
                    kind,
                    all,
                    changes: change_set,
                    origin,
                };
                let result = catch_unwind(AssertUnwindSafe(|| run_setup(plugin, &mut backend, state)));
                let read = backend.inputs.into_inner();
                let (result, state, time) = match result {
                    Ok(result) => result,
                    Err(panic) => {
                        report.fatal.push(format!(
                            "plugin `{}` panicked in setup: {}",
                            plugin.info.name,
                            panic_message(&*panic)
                        ));
                        continue;
                    }
                };
                PluginHost::add_time(&mut times, index, HookKind::Setup, time);
                report.build_inputs.extend(read.iter().cloned());
                match result {
                    Ok(made) => {
                        shared.push(made);
                        states.push(state);
                        inputs.push(read);
                    }
                    Err(fatal) => report.fatal.push(format!("plugin `{}`: {fatal}", plugin.info.name)),
                }
            }
            run = Some(PluginRun { active, shared, states, inputs, times });
        }
        report.timings.plugin_setup = start.elapsed();
        if !report.fatal.is_empty() {
            return (report, summary, committed, Some(loaded));
        }
        if check_failed {
            if let Some(run) = &run {
                explain_failures(&mut report, project, services, &run.active, &run.shared, &run.times, &ctx);
            }
            return (report, summary, committed, Some(loaded));
        }

        // 5. transform
        let claims_all = self.plugins.claims_all || std::env::var("RBXTS_PLUGIN_CLAIMS").is_ok_and(|v| v == "all");
        let start = Instant::now();
        let mut asts: Vec<Option<Vec<Stmt>>> = Vec::with_capacity(to_transform.len());
        let mut new_parts: Vec<Vec<Option<ErasedPart>>> = Vec::with_capacity(to_transform.len());
        let mut file_late_keys: Vec<Vec<(usize, String)>> = Vec::with_capacity(to_transform.len());
        let mut load_checks = Vec::with_capacity(to_transform.len());
        let mut hook_times = None;
        {
            let mut checker = project.program.get_type_checker(&ctx);
            let macro_manager = match MacroManager::new(&mut checker) {
                Ok(manager) => manager,
                Err(error) => {
                    report.fatal = error.messages;
                    return (report, summary, committed, Some(loaded));
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
            if let Some(host) = &host
                && !host.unsupported_interests.is_empty()
            {
                report.fatal.extend(host.unsupported_interests.iter().cloned());
                return (report, summary, committed, Some(loaded));
            }
            let mut multi = MultiTransformState::new();
            for &file in &to_transform {
                let transformed =
                    transform_file(project, &mut checker, services, &macro_manager, &mut multi, host.as_ref(), file);
                let file_name = file.get().file_name().to_owned();
                let out_path = output_path(services, &file_name, self.options.out_dir_override.as_deref());
                new_parts.push(transformed.parts);
                file_late_keys.push(transformed.late_keys);
                load_checks.push(transformed.load_check);
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
        if any_errors && !self.options.write_partial {
            if let Some(run) = &run {
                record_timings(&mut report, &run.active, &run.times, hook_times.as_ref());
            }
            return (report, summary, committed, Some(loaded));
        }

        // 6. plugin finish: the parts of every current project file, in program order
        let start = Instant::now();
        let file_indices: FxHashMap<P<SourceFile>, FileIndex> =
            project.program.source_files().iter().enumerate().map(|(i, &f)| (f, FileIndex(i as u32))).collect();
        let transformed_at: FxHashMap<P<SourceFile>, usize> =
            to_transform.iter().enumerate().map(|(i, &f)| (f, i)).collect();
        let mut late_strings = LateStrings::new();
        let mut queue = CommitQueue::default();
        let mut finished = run.is_none();
        if !any_errors && let Some(run) = run.as_mut() {
            let mut luau_outputs = FxHashSet::default();
            for &file in &files {
                let name = file.get().file_name();
                luau_outputs.insert(output_key(
                    services,
                    &output_path(services, name, self.options.out_dir_override.as_deref()),
                ));
                luau_outputs.insert(output_key(services, &services.path_translator.get_output_path(name)));
            }
            // the rest of the output set too (assets, declaration and transformed outputs): an artifact never
            // overwrites a copied asset
            for path in &project_outputs.files {
                luau_outputs.insert(output_key(services, path));
            }
            let mut resolved = FxHashSet::default();
            let mut errors = Vec::new();
            for (index, plugin) in run.active.iter().enumerate() {
                let mut plugin_parts: Vec<(FileIndex, &ErasedPart)> = Vec::with_capacity(files.len());
                for &file in &files {
                    let part = match transformed_at.get(&file) {
                        Some(&t) => new_parts[t].get(index).and_then(Option::as_ref),
                        None => committed
                            .as_ref()
                            .and_then(|c| c.files.get(file.path().as_str()))
                            .and_then(|c| c.parts.get(index))
                            .and_then(Option::as_ref),
                    };
                    if let Some(part) = part {
                        plugin_parts.push((file_indices[&file], part));
                    }
                }
                let mut backend = Finish {
                    program: project.program,
                    services,
                    plugin: plugin.info.name,
                    out_dir: &report.out_dir,
                    luau_outputs: &luau_outputs,
                    late: &mut late_strings,
                    resolved: &mut resolved,
                    queue: &mut queue,
                    errors: &mut errors,
                    warnings: &mut report.warnings,
                    messages: &mut report.plugin_messages,
                    kind,
                    all,
                };
                let result = catch_unwind(AssertUnwindSafe(|| {
                    run_finish(plugin, &run.shared[index], &mut run.states[index], &plugin_parts, &mut backend)
                }));
                let (result, time) = match result {
                    Ok(result) => result,
                    Err(panic) => {
                        errors.push(format!(
                            "plugin `{}` panicked in finish: {}",
                            plugin.info.name,
                            panic_message(&*panic)
                        ));
                        continue;
                    }
                };
                PluginHost::add_time(&mut run.times, index, HookKind::Finish, time);
                if let Err(fatal) = result {
                    errors.push(format!("plugin `{}`: {fatal}", plugin.info.name));
                }
            }
            // every late key a current file uses must have a value, cached files included
            let mut reported = FxHashSet::default();
            for &file in &files {
                let keys: Vec<(usize, &str)> = match transformed_at.get(&file) {
                    Some(&t) => file_late_keys[t].iter().map(|(p, k)| (*p, k.as_str())).collect(),
                    None => committed
                        .as_ref()
                        .and_then(|c| c.files.get(file.path().as_str()))
                        .map(|c| c.late.iter().map(|(p, k, _)| (*p, k.as_str())).collect())
                        .unwrap_or_default(),
                };
                for (plugin, key) in keys {
                    if !resolved.contains(key) && reported.insert(key.to_owned()) {
                        let relative =
                            nodepath::relative(&services.data.project_path, file.get().file_name()).replace('\\', "/");
                        errors.push(format!(
                            "plugin `{}` left the late string `{}` unresolved (used in {relative})",
                            run.active[plugin].info.name,
                            plain_late_key(key)
                        ));
                    }
                }
            }
            report.fatal.extend(errors);
            finished = report.fatal.is_empty();
        }
        report.timings.plugin_finish = start.elapsed();
        if let Some(run) = &run {
            record_timings(&mut report, &run.active, &run.times, hook_times.as_ref());
        }
        if !report.fatal.is_empty() && !self.options.write_partial {
            return (report, summary, committed, Some(loaded));
        }

        // 7. render, and re-render kept files whose late strings changed value
        let start = Instant::now();
        let mut statement_lines: Vec<Option<Vec<u32>>> = Vec::with_capacity(report.files.len());
        for ((outcome, ast), (keys, check)) in
            report.files.iter_mut().zip(&asts).zip(file_late_keys.iter().zip(&load_checks))
        {
            statement_lines.push(None);
            if let Some(ast) = ast
                && outcome.is_clean()
                && (keys.is_empty() || finished)
            {
                // rbxts-rs: a panic while rendering is this file's internal error, not the build's crash; a file
                // marked for the load check keeps where each statement starts, to place Luau's messages
                let rendered = quietly(|| match check {
                    Some(_) => {
                        let (luau, lines) = rbxts_luau::render_with_lines(ast, &late_strings);
                        (luau, Some(lines))
                    }
                    None => (rbxts_luau::render(ast, &late_strings), None),
                });
                match rendered {
                    Ok((luau, lines)) => {
                        outcome.luau = Some(luau);
                        *statement_lines.last_mut().expect("pushed above") = lines;
                    }
                    Err(panic) => {
                        let message = format!("rendering the Luau panicked: {}", panic_message(&*panic));
                        match project.program.get_source_file(&outcome.file_name) {
                            Some(file) => outcome.diagnostics.push(internal_error(file.as_node(), &message)),
                            None => {
                                report.fatal.push(format!("{}: rbxts-rs internal error: {message}", outcome.file_name))
                            }
                        }
                        outcome.internal_errors.push(message);
                    }
                }
            }
        }
        let mut rerendered: Vec<(String, String)> = Vec::new();
        // per re-rendered file with a load-check mark: (index in `rerendered`, its file, statement lines, mark)
        let mut rerendered_checks: Vec<(usize, P<SourceFile>, Vec<u32>, &LoadCheck)> = Vec::new();
        if finished && let Some(c) = &committed {
            for &file in &files {
                if transformed_at.contains_key(&file) {
                    continue;
                }
                let Some(cached) = c.files.get(file.path().as_str()) else { continue };
                if cached.late.iter().all(|(_, key, value)| late_strings.get(key) == Some(value.as_str())) {
                    continue;
                }
                let name = file.get().file_name();
                let Some(ast) = &cached.ast else {
                    report.fatal.push(format!(
                        "internal error: the late strings of {name} changed value, but its Luau was not kept"
                    ));
                    continue;
                };
                // rbxts-rs: rendered as a transformed file is, under the panic guard and with statement lines when a
                // plugin emitted into it (the load check runs on it below)
                match quietly(|| rbxts_luau::render_with_lines(ast, &late_strings)) {
                    Ok((luau, lines)) => {
                        if let Some(check) = &cached.load_check {
                            rerendered_checks.push((rerendered.len(), file, lines, check));
                        }
                        rerendered.push((output_path(services, name, self.options.out_dir_override.as_deref()), luau));
                    }
                    Err(panic) => {
                        let message = format!("rendering the Luau panicked: {}", panic_message(&*panic));
                        report.diagnostics.push(internal_error(file.as_node(), &message));
                    }
                }
            }
        }
        report.timings.render = start.elapsed();
        // Luau's compiler on every file a plugin emitted into, in parallel: an error keeps the file from being written
        let start = Instant::now();
        let checked: Vec<usize> = (0..report.files.len())
            .filter(|&i| load_checks[i].is_some() && statement_lines[i].is_some() && report.files[i].luau.is_some())
            .collect();
        // the transformed files, then the cached files re-rendered with new late-string values
        let jobs: Vec<(&str, bool)> = checked
            .iter()
            .map(|&i| {
                let luau = report.files[i].luau.as_deref().expect("filtered above");
                (luau, load_checks[i].as_ref().expect("filtered above").has_directive())
            })
            .chain(
                rerendered_checks.iter().map(|(at, _, _, check)| (rerendered[*at].1.as_str(), check.has_directive())),
            )
            .collect();
        let verdicts = crate::plugins::spill::decide_all(&jobs);
        let (transformed_verdicts, rerendered_verdicts) = verdicts.split_at(checked.len());
        for (&i, verdict) in checked.iter().zip(transformed_verdicts) {
            let (Some(check), Some(lines)) = (&load_checks[i], &statement_lines[i]) else { continue };
            if let Some(diagnostic) =
                crate::plugins::spill::load_check_diagnostic(check, verdict, lines, to_transform[i])
            {
                let outcome = &mut report.files[i];
                if diagnostic.category() == Category::Error {
                    outcome.luau = None;
                }
                outcome.diagnostics.push(diagnostic);
            }
        }
        // a re-rendered file has no outcome of its own: its diagnostics are the build's (an error stops the commit)
        for ((_, file, lines, check), verdict) in rerendered_checks.iter().zip(rerendered_verdicts) {
            if let Some(diagnostic) = crate::plugins::spill::load_check_diagnostic(check, verdict, lines, *file) {
                report.diagnostics.push(diagnostic);
            }
        }
        report.timings.load_checked = (checked.len() + rerendered_checks.len()) as u32;
        report.timings.load_check = start.elapsed();
        // a file that failed to render or Luau refuses fails the build as a transform error does
        if (!report.fatal.is_empty() || report.has_errors()) && !self.options.write_partial {
            return (report, summary, committed, Some(loaded));
        }

        // 8. commit
        let start = Instant::now();
        let success = !report.has_errors() && finished;
        // the assets' bytes are read before the first write, so a vanished asset fails the build with nothing written
        let asset_bytes = if success && !self.options.no_write {
            match outputs::read_assets(&project_outputs) {
                Ok(bytes) => bytes,
                Err(message) => {
                    report.fatal.push(message);
                    return (report, summary, committed, Some(loaded));
                }
            }
        } else {
            Vec::new()
        };
        // the runtime files (`copyInclude`), read before the first write too
        let include = match (success && !self.options.no_write)
            .then(|| prepare_copy_include(services, self.options.out_dir_override.as_deref()))
        {
            Some(Err(message)) => {
                report.fatal.push(message);
                return (report, summary, committed, Some(loaded));
            }
            prepared => prepared.and_then(Result::ok).flatten(),
        };
        if !self.options.no_write {
            for outcome in &mut report.files {
                let Some(source) = &outcome.luau else { continue };
                // rbxts-rs: a case-only rename renames the output too (roblox-ts keeps the old name: its bug)
                if let Err(e) = outputs::match_case(services, &outcome.out_path, &listing) {
                    report.fatal.push(format!("could not rename to {}: {e}", outcome.out_path));
                    continue;
                }
                match write_if_changed(Path::new(&outcome.out_path), source.as_bytes()) {
                    Ok(written) => {
                        outcome.written = written;
                        if written {
                            summary.written.push(outcome.out_path.clone());
                        }
                    }
                    Err(e) => report.fatal.push(format!("could not write {}: {e}", outcome.out_path)),
                }
            }
            if success && report.fatal.is_empty() {
                for (path, source) in &rerendered {
                    match write_if_changed(Path::new(path), source.as_bytes()) {
                        Ok(true) => summary.written.push(path.clone()),
                        Ok(false) => {}
                        Err(e) => report.fatal.push(format!("could not write {path}: {e}")),
                    }
                }
                summary.rerendered = rerendered.iter().map(|(p, _)| p.clone()).collect();
            }
            if success && report.fatal.is_empty() {
                let artifacts: Vec<String> = queue.artifacts.iter().map(|(_, p, _)| p.clone()).collect();
                let stale: Vec<String> = committed
                    .as_ref()
                    .map(|c| c.artifacts.iter().filter(|a| !artifacts.contains(a)).cloned().collect())
                    .unwrap_or_default();
                let disk = state_path.as_ref().map(|path| {
                    let state = DiskState {
                        compiler: state::compiler_identity(),
                        plugins: plugin_ids.clone(),
                        salt,
                        files: facts.iter().map(|(k, f)| (k.clone(), f.clone())).collect(),
                        artifacts: artifacts.clone(),
                    };
                    (path.as_str(), state)
                });
                if let Err(message) = self.commit_rest(
                    &mut report,
                    &mut summary,
                    &loaded,
                    &mut project_outputs,
                    &asset_bytes,
                    include.as_ref(),
                    &queue,
                    &stale,
                    &listing,
                    disk,
                ) {
                    report.fatal.push(message);
                }
            }
        }
        report.timings.write = start.elapsed();
        if !success || !report.fatal.is_empty() {
            return (report, summary, committed, Some(loaded));
        }

        // 9. the new committed state
        let run_parts = run.map(|run| {
            (run.active.iter().map(|p| p.info.name).collect::<Vec<_>>(), run.shared, run.states, run.inputs)
        });
        let mut old_files = committed.map(|c| c.files).unwrap_or_default();
        let mut new_files = FxHashMap::default();
        let mut parts_by_file: FxHashMap<P<SourceFile>, Vec<Option<ErasedPart>>> = FxHashMap::default();
        for (&file, parts) in to_transform.iter().zip(new_parts) {
            parts_by_file.insert(file, parts);
        }
        let mut asts_by_file: FxHashMap<P<SourceFile>, Option<Vec<Stmt>>> =
            to_transform.iter().copied().zip(asts).collect();
        let late_by_file: FxHashMap<P<SourceFile>, Vec<(usize, String)>> =
            to_transform.iter().copied().zip(file_late_keys).collect();
        let mut checks_by_file: FxHashMap<P<SourceFile>, Option<LoadCheck>> =
            to_transform.iter().copied().zip(load_checks).collect();
        let program_files: FxHashMap<String, P<SourceFile>> =
            project.program.source_files().iter().map(|&f| (f.path().to_string(), f)).collect();
        for (key, facts) in facts {
            let file = program_files.get(&key).copied();
            let entry = match file.and_then(|f| parts_by_file.remove(&f).map(|p| (f, p))) {
                Some((file, parts)) => {
                    let late: Vec<(usize, String, String)> = late_by_file[&file]
                        .iter()
                        .map(|(p, k)| (*p, k.clone(), late_strings.get(k).unwrap_or_default().to_owned()))
                        .collect();
                    let ast = if self.watch && !late.is_empty() { asts_by_file.remove(&file).flatten() } else { None };
                    let load_check = if ast.is_some() { checks_by_file.remove(&file).flatten() } else { None };
                    CommittedFile { facts, parts, late, ast, load_check }
                }
                None => match old_files.remove(&key) {
                    Some(mut old) => {
                        old.facts = facts;
                        for (_, k, value) in &mut old.late {
                            if let Some(v) = late_strings.get(k) {
                                *value = v.to_owned();
                            }
                        }
                        old
                    }
                    None => CommittedFile { facts, parts: Vec::new(), late: Vec::new(), ast: None, load_check: None },
                },
            };
            new_files.insert(key, entry);
        }
        let plugins = match run_parts {
            Some((names, shared, states, inputs)) => names
                .into_iter()
                .zip(shared)
                .zip(states)
                .zip(inputs)
                .map(|(((name, shared), state), inputs)| PluginRecord { name, state, shared: Some(shared), inputs })
                .collect(),
            None => Vec::new(),
        };
        let committed = Committed {
            salt,
            plugin_ids,
            files: new_files,
            plugins,
            artifacts: queue.artifacts.iter().map(|(_, p, _)| p.clone()).collect(),
            watch_dirs: queue.watch_dirs.iter().map(|(_, p)| p.clone()).collect(),
            from_disk: false,
        };
        summary.changes = changes;
        self.pending.clear();
        (report, summary, Some(committed), Some(loaded))
    }

    /// Commit steps 1 (assets, the runtime include files), 2, 3 and 4 after the Luau outputs.
    #[allow(clippy::too_many_arguments)]
    fn commit_rest(
        &self,
        report: &mut CompileReport,
        summary: &mut BuildSummary,
        loaded: &Loaded,
        outputs: &mut outputs::ProjectOutputs,
        asset_bytes: &[Vec<u8>],
        include: Option<&IncludeCopy>,
        queue: &CommitQueue,
        stale: &[String],
        listing: &outputs::Listing,
        disk: Option<(&str, DiskState)>,
    ) -> Result<(), String> {
        let services = &loaded.services;
        // this build's artifacts, state files and incremental state are kept too
        for path in queue
            .artifacts
            .iter()
            .map(|(_, p, _)| p.as_str())
            .chain(queue.state_files.iter().map(|(_, p, _)| p.as_str()))
            .chain(disk.as_ref().map(|(path, _)| *path))
        {
            outputs.keep.insert(outputs::path_key(services, path));
        }
        // 1. assets (the Luau is written)
        summary.written.extend(outputs::sync_assets(services, outputs, asset_bytes, listing)?);
        if let Some(include) = include {
            summary.written.extend(include.commit()?);
        }
        // 2. artifacts
        let written = commit_artifacts(queue)?;
        summary.written.extend(written.iter().cloned());
        report.artifacts_written.extend(written);
        // 3. stale outputs, stale artifacts, removals
        let deleted = outputs::clean(services, outputs, listing)?;
        summary.deleted.extend(deleted);
        let deleted = commit_removals(queue, stale)?;
        summary.deleted.extend(deleted.iter().cloned());
        report.artifacts_deleted.extend(deleted);
        // 4. plugin state files, then the incremental state
        let written = commit_state_files(queue)?;
        summary.written.extend(written.iter().cloned());
        report.artifacts_written.extend(written);
        if let Some((path, state)) = disk
            && state.write(path).map_err(|e| format!("could not write the incremental state {path}: {e}"))?
        {
            summary.written.push(path.to_owned());
        }
        Ok(())
    }

    /// roblox-ts's `getProjectOutputs` for this build: the output roots, the keep set (without this build's plugin
    /// files) and the assets, or the error that stops the build before anything is written.
    fn project_outputs(&self, loaded: &Loaded, files: &[P<SourceFile>]) -> Result<outputs::ProjectOutputs, String> {
        let services = &loaded.services;
        let out_dir_override = self.options.out_dir_override.as_deref();
        let emitted: Vec<(String, String, Vec<String>)> = files
            .iter()
            .map(|f| {
                let name = f.get().file_name();
                let declarations = loaded
                    .project
                    .declaration_outputs(name)
                    .iter()
                    .map(|d| outputs::overridden(services, d, out_dir_override))
                    .collect();
                (name.to_owned(), output_path(services, name, out_dir_override), declarations)
            })
            .collect();
        let options = loaded.project.program.options();
        let ts_build_info = rbxts_project::setup::ts5_build_info_path(&options);
        outputs::get_project_outputs(&OutputInputs {
            services,
            options: &options,
            emitted: &emitted,
            transformed_outputs: services.data.options.write_transformed_files
                && options.plugins.as_ref().is_some_and(|p| !p.is_empty()),
            out_dir_override,
            config_paths: &loaded.config_paths,
            ts_build_info: ts_build_info.as_deref(),
        })
    }

    /// Where the incremental state goes: beside roblox-ts's build info for an `incremental` or `composite`
    /// project, and never for parity runs (an outDir override) or builds that write nothing.
    fn state_path(&self, services: &ProjectServices) -> Option<String> {
        if self.options.out_dir_override.is_some() || self.options.no_write || self.options.write_partial {
            return None;
        }
        services.path_translator.build_info_output_path.as_deref().map(state::state_path)
    }

    /// The working state a plugin's `setup` starts from.
    fn initial_state(&self, plugin: &ActivePlugin<'_>, kind: BuildKind, committed: Option<&Committed>) -> ErasedState {
        if let Some(seed) = &self.seed
            && let Some((_, state)) = seed.iter().find(|(name, _)| *name == plugin.info.name)
        {
            return state.clone();
        }
        if matches!(kind, BuildKind::WatchRebuild { .. })
            && let Some(record) = committed.and_then(|c| c.plugins.iter().find(|r| r.name == plugin.info.name))
        {
            return record.state.clone();
        }
        plugin.plugin.default_state()
    }

    /// The config inputs' path keys: tsconfig and `extends`, Rojo project files, plugin input files.
    fn config_inputs_of(&self, loaded: &Loaded, committed: Option<&Committed>) -> FxHashSet<String> {
        let services = &loaded.services;
        let mut keys: FxHashSet<String> = loaded.config_paths.iter().map(|p| path_key_of(services, p)).collect();
        keys.extend(services.rojo_resolver.config_files().iter().map(|p| path_key_of(services, p)));
        let committed = committed.or(self.committed.as_ref());
        if let Some(c) = committed {
            for plugin in &c.plugins {
                for input in &plugin.inputs {
                    if let BuildInput::File { path, .. } = input {
                        keys.insert(path_key_of(services, path));
                    }
                }
            }
        }
        keys
    }
}

/// A path's comparison key.
fn path_key_of(services: &ProjectServices, path: &str) -> String {
    outputs::path_key(services, path)
}

/// The plugins of a build after setup.
struct PluginRun<'p> {
    active: Vec<ActivePlugin<'p>>,
    shared: Vec<ErasedShared>,
    states: Vec<ErasedState>,
    inputs: Vec<Vec<BuildInput>>,
    times: Vec<[HookTime; HookKind::ALL.len()]>,
}

/// A build input changed since `setup` read it.
fn input_changed(
    input: &BuildInput,
    program: &'static tsrs_compiler::Program,
    resolver: &OnceCell<rbxts_project::ModuleResolver>,
) -> bool {
    match input {
        BuildInput::File { path, version, .. } => {
            std::fs::read_to_string(path).ok().map(|text| Version::of(&text)) != *version
        }
        BuildInput::Env { name, value, .. } => std::env::var(name).ok() != *value,
        BuildInput::Resolution { specifier, from, resolved, .. } => {
            resolve_module_from(program, resolver, specifier, from).map(|r| r.file_name) != *resolved
        }
    }
}
