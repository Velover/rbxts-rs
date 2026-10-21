//! The build lifecycle around the transform (DESIGN §4, §14): which plugins are on, `setup` and `finish` with their
//! backends, the late-string key check, the commit, and `explain_checker_diagnostic` on files that failed checking.
//!
//! The build's kind, changes, state origin and whether every file is transformed come from the build session
//! ([`crate::build`]), which also keeps the `Shared`s and committed states across builds, the cached file parts
//! and late values, and the artifact set of the last build (stale deletions).

use std::cell::{OnceCell, RefCell};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustc_hash::FxHashSet;
use serde_json::{Map, Value};

use rbxts_luau::LateStrings;
use rbxts_plugin::erased::{DynPlugin, ErasedPart, ErasedShared, ErasedState};
use rbxts_plugin::{
    BuildKind, ChangeSet, FileIndex, FinishBackend, FinishCx, PluginInfo, SetupBackend, SetupCx, StateOrigin, Utf16Pos,
};
use rbxts_project::{ModuleResolver, ProjectServices, ResolvedModuleInfo, nodepath};
use tsrs_compiler::Program;
use tsrs_core::{CompilerOptions, P};

use super::HookKind;
use super::host::{HookTime, resolve_module_from};

/// The plugins of a build and the switches that go with them.
#[derive(Default)]
pub struct PluginOptions<'p> {
    /// Registered plugins, in registration order (the claim order).
    pub plugins: &'p [Box<dyn DynPlugin>],
    /// A tsconfig `plugins` entry whose `transform` no registered plugin claims is an error ("not compiled into
    /// this toolchain"); with this set it is ignored (measurement runs on a project's own tsconfig).
    pub ignore_unknown_tsconfig_plugins: bool,
    /// `RBXTS_PLUGIN_CLAIMS=all`: ask every matching plugin, and report a double claim as an error. The
    /// environment variable turns it on too.
    pub claims_all: bool,
}

/// A registered plugin that is on for this project.
pub(crate) struct ActivePlugin<'p> {
    pub plugin: &'p dyn DynPlugin,
    pub info: PluginInfo,
    /// Its tsconfig `plugins` entry.
    pub entry: Option<Map<String, Value>>,
}

/// What a plugin read in `setup`: recorded so incremental and watch builds can rebuild when one changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildInput {
    /// A file read through `read_input` (its absolute path, whether it existed, and the version of its text).
    File { plugin: &'static str, path: String, exists: bool, version: Option<crate::build::graph::Version> },
    /// An environment variable read through `env`.
    Env { plugin: &'static str, name: String, value: Option<String> },
    /// A `resolve_module` from `setup`.
    Resolution { plugin: &'static str, specifier: String, from: String, resolved: Option<String> },
}

/// One plugin's hook times.
#[derive(Clone, Debug)]
pub struct PluginTimings {
    pub name: &'static str,
    /// Per hook kind: calls and time, without the compiler work hooks asked for.
    pub hooks: Vec<(HookKind, HookTime)>,
}

/// The late-string key of a plugin's key: keys are per plugin.
pub(crate) fn late_key(plugin: &str, key: &str) -> String {
    format!("{plugin}\u{0}{key}")
}

/// The plugin's own key of a namespaced late-string key.
pub(crate) fn plain_late_key(key: &str) -> &str {
    key.split_once('\u{0}').map_or(key, |(_, plain)| plain)
}

/// The registered plugins that are on for the project: `tsconfig_transform: None`, or a tsconfig `plugins` entry
/// with that `transform`. Errors: an entry no registered plugin claims (unless ignored), two plugins with one name.
pub(crate) fn activate<'p>(
    options: &PluginOptions<'p>,
    tsconfig_path: &str,
) -> Result<Vec<ActivePlugin<'p>>, Vec<String>> {
    let entries = rbxts_project::setup::read_tsconfig_plugins(tsconfig_path).map_err(|e| e.messages)?;
    let mut errors = Vec::new();
    let mut names = FxHashSet::default();
    for plugin in options.plugins {
        let name = plugin.info().name;
        if !names.insert(name) {
            errors.push(format!("two plugins are named `{name}`: plugin names must be unique"));
        }
    }
    for entry in &entries {
        let Some(transform) = entry.get("transform").and_then(Value::as_str) else { continue };
        let claimed = options.plugins.iter().any(|p| p.info().tsconfig_transform == Some(transform));
        if !claimed && !options.ignore_unknown_tsconfig_plugins {
            errors.push(format!(
                "tsconfig plugin \"{transform}\" is not compiled into this toolchain (no registered plugin has \
                 that tsconfig_transform)"
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut active = Vec::new();
    for plugin in options.plugins {
        let info = plugin.info();
        let entry = match info.tsconfig_transform {
            None => None,
            Some(transform) => {
                match entries.iter().find(|e| e.get("transform").and_then(Value::as_str) == Some(transform)) {
                    Some(entry) => Some(entry.clone()),
                    None => continue,
                }
            }
        };
        active.push(ActivePlugin { plugin: &**plugin, info, entry });
    }
    Ok(active)
}

/// A path a plugin named: absolute, or relative to the project directory.
fn plugin_path(services: &ProjectServices, path: &Path) -> String {
    let text = path.to_string_lossy();
    if nodepath::is_absolute(&text) {
        nodepath::normalize(&text)
    } else {
        nodepath::resolve(&[&services.data.project_path, &text])
    }
}

/// A path as a comparison key (case-insensitive unless the file system is case-sensitive).
fn path_key(services: &ProjectServices, path: &str) -> String {
    let normalized = nodepath::normalize(path).replace('\\', "/");
    if services.use_case_sensitive_file_names { normalized } else { normalized.to_lowercase() }
}

/// A component Windows would rewrite when opening it (a trailing `.` or space), so that two spellings name one
/// file: `a.luau.` is `a.luau`.
fn has_rewritten_component(path: &str) -> bool {
    path.split(['/', '\\']).any(|c| c != "." && c != ".." && (c.ends_with('.') || c.ends_with(' ')))
}

/// The path as the file system resolves it: its deepest existing ancestor canonicalised (junctions, symlinks,
/// case), the rest appended. A comparison key like [`path_key`].
fn real_key(services: &ProjectServices, path: &str) -> String {
    let mut existing = PathBuf::from(nodepath::normalize(path));
    let mut rest = Vec::new();
    while !existing.exists() {
        match existing.file_name() {
            Some(name) => {
                rest.push(name.to_owned());
                existing.pop();
            }
            None => break,
        }
    }
    let mut real = std::fs::canonicalize(&existing).unwrap_or(existing);
    for name in rest.iter().rev() {
        real.push(name);
    }
    let text = real.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    path_key(services, text)
}

// --- setup ---

/// The compiler's side of `SetupCx` for one plugin.
pub(crate) struct Setup<'s> {
    pub program: &'static Program,
    pub services: &'s ProjectServices,
    pub plugin: &'static str,
    pub entry: Option<&'s Map<String, Value>>,
    pub resolver: &'s OnceCell<ModuleResolver>,
    /// What setup read (a `RefCell`: `resolve_module` takes `&self`).
    pub inputs: RefCell<Vec<BuildInput>>,
    pub messages: &'s mut Vec<String>,
    pub warnings: &'s mut Vec<String>,
    pub kind: BuildKind,
    /// Every source file is transformed this build.
    pub all: bool,
    /// What changed since the last committed build (`None` for a clean build).
    pub changes: Option<&'s ChangeSet>,
    pub origin: StateOrigin,
}

impl SetupBackend for Setup<'_> {
    fn program(&self) -> &'static Program {
        self.program
    }

    fn options(&self) -> P<CompilerOptions> {
        self.program.options()
    }

    fn project(&self) -> &ProjectServices {
        self.services
    }

    fn tsconfig_entry(&self) -> Option<&Map<String, Value>> {
        self.entry
    }

    fn resolve_module(&self, specifier: &str, from_file: &str) -> Option<ResolvedModuleInfo> {
        let resolved = resolve_module_from(self.program, self.resolver, specifier, from_file);
        // a build input: a package installed mid-watch must trigger a rebuild
        self.inputs.borrow_mut().push(BuildInput::Resolution {
            plugin: self.plugin,
            specifier: specifier.to_owned(),
            from: from_file.to_owned(),
            resolved: resolved.as_ref().map(|r| r.file_name.clone()),
        });
        resolved
    }

    fn read_input(&mut self, path: &Path) -> io::Result<Option<String>> {
        let path = plugin_path(self.services, path);
        let result = match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        };
        let exists = matches!(result, Ok(Some(_)));
        let version = match &result {
            Ok(Some(text)) => Some(crate::build::graph::Version::of(text)),
            _ => None,
        };
        self.inputs.borrow_mut().push(BuildInput::File { plugin: self.plugin, path, exists, version });
        result
    }

    fn env(&mut self, name: &str) -> Option<String> {
        let value = std::env::var(name).ok();
        self.inputs.borrow_mut().push(BuildInput::Env {
            plugin: self.plugin,
            name: name.to_owned(),
            value: value.clone(),
        });
        value
    }

    fn build_kind(&self) -> BuildKind {
        self.kind
    }

    fn transforms_all_files(&self) -> bool {
        self.all
    }

    fn changes(&self) -> Option<&ChangeSet> {
        self.changes
    }

    fn state_origin(&self) -> StateOrigin {
        self.origin
    }

    fn read_persisted(&mut self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(plugin_path(self.services, path)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn info(&mut self, message: &str) {
        self.messages.push(format!("{}: {message}", self.plugin));
    }

    fn warn(&mut self, message: &str, hint: Option<&str>) {
        self.warnings.push(match hint {
            Some(hint) => format!("{}: {message}\n{hint}", self.plugin),
            None => format!("{}: {message}", self.plugin),
        });
    }
}

/// Runs one plugin's `setup` on `state`: `default_state()` for a clean or incremental build (the plugin reloads
/// what it persisted itself with `read_persisted`), a copy of the committed state for a watch rebuild.
pub(crate) fn run_setup(
    active: &ActivePlugin<'_>,
    backend: &mut Setup<'_>,
    mut state: ErasedState,
) -> (Result<ErasedShared, rbxts_plugin::Fatal>, ErasedState, Duration) {
    let start = std::time::Instant::now();
    let shared = active.plugin.setup(&mut SetupCx::new(backend), &mut state);
    (shared, state, start.elapsed())
}

// --- finish ---

/// What `finish` queued for the commit.
#[derive(Default)]
pub(crate) struct CommitQueue {
    /// (plugin, absolute path, bytes): written in step 2, only if changed.
    pub artifacts: Vec<(&'static str, String, Vec<u8>)>,
    /// Written in step 4.
    pub state_files: Vec<(&'static str, String, Vec<u8>)>,
    /// Deleted in step 3.
    pub removals: Vec<(&'static str, String)>,
    /// Glob roots to watch.
    pub watch_dirs: Vec<(&'static str, String)>,
}

/// The compiler's side of `FinishCx` for one plugin.
pub(crate) struct Finish<'s> {
    pub program: &'static Program,
    pub services: &'s ProjectServices,
    pub plugin: &'static str,
    /// Where outputs go (the override when there is one).
    pub out_dir: &'s str,
    /// The path keys of this build's Luau outputs.
    pub luau_outputs: &'s FxHashSet<String>,
    pub late: &'s mut LateStrings,
    pub resolved: &'s mut FxHashSet<String>,
    pub queue: &'s mut CommitQueue,
    pub errors: &'s mut Vec<String>,
    pub warnings: &'s mut Vec<String>,
    pub messages: &'s mut Vec<String>,
    pub kind: BuildKind,
    pub all: bool,
}

impl Finish<'_> {
    /// Checks an artifact or state-file path: under the project directory, the outDir or the include path, and not
    /// a Luau output. Records an error naming the plugin otherwise.
    fn checked_path(&mut self, path: &Path, what: &str) -> Option<String> {
        let absolute = plugin_path(self.services, path);
        if has_rewritten_component(&absolute) {
            self.errors.push(format!(
                "plugin `{}`: {what} {absolute} has a path component ending in `.` or a space",
                self.plugin
            ));
            return None;
        }
        // through the file system: a junction or symlink inside the project can lead outside it
        let key = real_key(self.services, &absolute);
        let data = &self.services.data;
        let roots = [
            data.project_path.as_str(),
            self.services.path_translator.out_dir.as_str(),
            self.out_dir,
            data.options.include_path.as_str(),
        ];
        let under_root = roots.iter().filter(|r| !r.is_empty()).any(|root| {
            let root = real_key(self.services, root);
            key.len() > root.len() && key.starts_with(&root) && key.as_bytes()[root.len()] == b'/'
        });
        if !under_root {
            self.errors.push(format!(
                "plugin `{}`: {what} {absolute} is outside the project directory, the outDir and the include path",
                self.plugin
            ));
            return None;
        }
        if self.luau_outputs.contains(&key) {
            self.errors.push(format!("plugin `{}`: {what} {absolute} is a Luau output of this build", self.plugin));
            return None;
        }
        Some(absolute)
    }
}

impl FinishBackend for Finish<'_> {
    fn project(&self) -> &ProjectServices {
        self.services
    }

    fn build_kind(&self) -> BuildKind {
        self.kind
    }

    fn transforms_all_files(&self) -> bool {
        self.all
    }

    fn resolve_late(&mut self, key: &str, escaped: String) {
        let key = late_key(self.plugin, key);
        self.resolved.insert(key.clone());
        self.late.insert(key, escaped);
    }

    fn write_artifact(&mut self, path: &Path, bytes: Vec<u8>) {
        if let Some(path) = self.checked_path(path, "artifact") {
            self.queue.artifacts.retain(|(_, p, _)| *p != path);
            self.queue.artifacts.push((self.plugin, path, bytes));
        }
    }

    fn write_state_file(&mut self, path: &Path, bytes: Vec<u8>) {
        if let Some(path) = self.checked_path(path, "state file") {
            self.queue.state_files.retain(|(_, p, _)| *p != path);
            self.queue.state_files.push((self.plugin, path, bytes));
        }
    }

    fn remove_artifact(&mut self, path: &Path) {
        if let Some(path) = self.checked_path(path, "artifact") {
            self.queue.removals.push((self.plugin, path));
        }
    }

    fn watch_dir(&mut self, path: &Path) {
        if let Some(path) = self.checked_path(path, "watched directory") {
            self.queue.watch_dirs.push((self.plugin, path));
        }
    }

    fn warn_at(&mut self, file: FileIndex, pos: Utf16Pos, message: &str, hint: Option<&str>) {
        let name = self.program.source_files().get(file.0 as usize).map_or("<unknown file>", |f| f.get().file_name());
        let relative = nodepath::relative(&self.services.data.project_path, name).replace('\\', "/");
        let mut text = format!("{relative}:{}:{}: {}: {message}", pos.line + 1, pos.character + 1, self.plugin);
        if let Some(hint) = hint {
            text.push('\n');
            text.push_str(hint);
        }
        self.warnings.push(text);
    }

    fn info(&mut self, message: &str) {
        self.messages.push(format!("{}: {message}", self.plugin));
    }
}

/// Runs one plugin's `finish` with the parts of every file, sorted by `FileIndex`.
pub(crate) fn run_finish(
    active: &ActivePlugin<'_>,
    shared: &ErasedShared,
    state: &mut ErasedState,
    parts: &[(FileIndex, &ErasedPart)],
    backend: &mut Finish<'_>,
) -> (Result<(), rbxts_plugin::Fatal>, Duration) {
    let start = std::time::Instant::now();
    let result = active.plugin.finish(shared, state, parts, &mut FinishCx::new(backend));
    (result, start.elapsed())
}

// --- commit ---

/// Whether an I/O error is worth retrying: a file another process holds open (Windows sharing and lock
/// violations, access denied while a scanner or editor has it).
fn is_transient(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied || matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

/// Writes `bytes` to a temporary file next to `path` and renames it into place (`MoveFileExW` with
/// `MOVEFILE_REPLACE_EXISTING`), retrying with backoff while another process holds the target. The error is
/// returned after the last attempt; nothing is skipped silently.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp: PathBuf = dir.join(format!(".{name}.rbxts-tmp-{}-{n}", std::process::id()));
    if let Err(e) = std::fs::write(&temp, bytes) {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    let mut delay = Duration::from_millis(10);
    let mut attempt = 0;
    loop {
        match std::fs::rename(&temp, path) {
            Ok(()) => return Ok(()),
            Err(e) if is_transient(&e) && attempt < 8 => {
                attempt += 1;
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(500));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                return Err(e);
            }
        }
    }
}

/// [`write_atomic`] when the file is missing or its bytes differ. Returns whether it wrote.
pub fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<bool> {
    if std::fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(false);
    }
    write_atomic(path, bytes)?;
    Ok(true)
}

/// Deletes a file, retrying while another process holds it; a missing file is fine.
pub fn remove_file(path: &Path) -> io::Result<bool> {
    let mut delay = Duration::from_millis(10);
    let mut attempt = 0;
    loop {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) if is_transient(&e) && attempt < 8 => {
                attempt += 1;
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(500));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Commit step 2: the plugin artifacts, each only if its bytes changed. Returns the paths written, or the first
/// failure (which stops the commit there).
pub(crate) fn commit_artifacts(queue: &CommitQueue) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for (plugin, path, bytes) in &queue.artifacts {
        match write_if_changed(Path::new(path), bytes) {
            Ok(true) => written.push(path.clone()),
            Ok(false) => {}
            Err(e) => return Err(format!("plugin `{plugin}`: could not write {path}: {e}")),
        }
    }
    Ok(written)
}

/// Commit step 3 for plugins: `remove_artifact`s, and the artifacts the last build wrote but this one did not
/// (`stale`, which the caller diffs). Returns the paths deleted.
pub(crate) fn commit_removals(queue: &CommitQueue, stale: &[String]) -> Result<Vec<String>, String> {
    let mut deleted = Vec::new();
    let removals = queue.removals.iter().map(|(plugin, path)| (*plugin, path)).chain(stale.iter().map(|p| ("", p)));
    for (plugin, path) in removals {
        match remove_file(Path::new(path)) {
            Ok(true) => deleted.push(path.clone()),
            Ok(false) => {}
            Err(e) if plugin.is_empty() => return Err(format!("could not delete the stale artifact {path}: {e}")),
            Err(e) => return Err(format!("plugin `{plugin}`: could not delete {path}: {e}")),
        }
    }
    Ok(deleted)
}

/// Commit step 4 for plugins: the state files, each only if its bytes changed. Returns the paths written.
pub(crate) fn commit_state_files(queue: &CommitQueue) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for (plugin, path, bytes) in &queue.state_files {
        match write_if_changed(Path::new(path), bytes) {
            Ok(true) => written.push(path.clone()),
            Ok(false) => {}
            Err(e) => return Err(format!("plugin `{plugin}`: could not write {path}: {e}")),
        }
    }
    Ok(written)
}

/// The key for a Luau output path, to keep artifacts off them.
pub(crate) fn output_key(services: &ProjectServices, path: &str) -> String {
    real_key(services, path)
}
