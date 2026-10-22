//! Watch mode (`rbxtsc -w`, DESIGN §14 "Watcher"): one [`BuildSession`] rebuilt on file system events.
//!
//! - **Events.** A `notify` watcher (ReadDirectoryChangesW on Windows, or polling with `--usePolling`). Watched:
//!   - recursively: the root directories, the directories plugins asked to watch (`watch_dir`), and every package
//!     directory the build depends on (a package a program file is in or a read went into, every package of a
//!     type root, a package an unresolved import names, a package plugin `resolve_module` calls went through);
//!   - not recursively: the project directory, the directories of program files outside packages,
//!     `node_modules` and its `@scope` directories and the type roots (created, removed or renamed, and the packages
//!     in them), and, for every path the last program load read (tsrs's `trackingvfs` view: files, directories
//!     and the paths module resolution probed that did not exist, so `node_modules/<name>` in every ancestor,
//!     `@types`, `typeRoots` and `paths` locations; setup's own reads: the project's package.json lookup, the
//!     type-root packages' package.json), the nearest existing directory at or above it. A missing `node_modules`
//!     counts too (a fresh clone, then an install). The folders the Rojo resolvers search for nested project files
//!     (`$path` folders outside the output roots) are watched recursively too.
//! - **What counts.** A JSON file the load read whose text is the same again never does (a package manager
//!   rewriting package.json as it was). Config files and build inputs always; nothing under the output roots (never
//!   a root holding the project, a source root or the tsconfig) or the include folder; then: a path the load read,
//!   or a missing directory on the way to one; an entry added, removed or renamed in a module directory (not dot
//!   entries: `.package-lock.json`, `.cache`); a Rojo project file in the project directory; in a folder a Rojo
//!   resolver searches (type roots included), a project file, or a folder holding one or holding one it read;
//!   inside a package the build uses, `package.json` and other JSON, TypeScript and Luau files and directories
//!   (nothing else in a package that is only in a type root); under a root (not in its `node_modules` or `.git`),
//!   anything; in the directory of a program file or one the include globs listed, a directory or a file the
//!   tsconfig could take in (lockfiles in the project directory never count). A directory's own modification never
//!   counts (its entries' events say more; a directory created or moved in does), nor do rbxts-rs's temporary files
//!   or this watcher's own artifacts as it wrote them.
//! - **New watches.** The watched directories are brought up to date after each build, before it is reported. A
//!   directory watched for the first time is compared with the build: a file written or created, or a directory
//!   created, since the build started; for a module, package or root directory its own modification time (an entry
//!   added, removed or renamed whatever the entry's times: an archive extracted with old times), unless this watcher
//!   wrote into it; a path the load read that is there now and was not, or the other way round; a program file
//!   whose time or size is not the one it was parsed at. Any of those starts another rebuild. A build that loads no
//!   project keeps the last good watches, plus what its failed load read. A watch that cannot be made is reported
//!   once and tried again at each update.
//! - **Plugin inputs** ([`inputs`]): the globs plugins recorded (files', `setup`'s, `finish`'s) and what `finish`
//!   read are watched too; an event only they may care about starts a rebuild when the session finds an answer
//!   changed.
//! - **Debounce.** Trailing, 100 ms as roblox-ts: each event restarts the wait, and the paths of one burst become
//!   one rebuild. A rename is its two paths (the session sees a deletion and an addition).
//! - **Errors.** A panic in a rebuild is reported and the session starts over with a clean build; the watcher
//!   keeps running.
//! - **`--events json`:** one JSON line per build on stdout: `{"event":"build-start","gen":N,"changes":[…]}` and
//!   `{"event":"build-end","gen":N,"ok":…,"errors":N,"written":[…],"deleted":[…]}`.
//! - **Memory.** Programs are never freed (tsrs's `free_program` is `unsafe`), so the watcher restarts its process
//!   past a working-set limit (`RBXTS_WATCH_MEMORY_LIMIT_MB`, default 2048; 0 turns it off): a small supervisor
//!   process runs the watcher as a child and starts a new one when it exits with [`RESTART_EXIT_CODE`]. The new
//!   watcher's first build is a clean one, or an incremental one for an `incremental` project.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, SystemTime};

use notify::event::ModifyKind;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_project::{IncludeMatcher, ReadLog, Seen, nodepath, rojo, text_hash};
use rbxts_transform::build::graph::Version;
use rbxts_transform::{BuildSession, BuildSummary, CompileOptions, CompileReport, PluginOptions};
use tsrs_diagnostics::Category;

use crate::{BuildArgs, print_report};

#[path = "watch_inputs.rs"]
mod inputs;

/// The exit code a watcher child uses to ask its supervisor for a restart.
pub const RESTART_EXIT_CODE: i32 = 75;
/// Set in a watcher child's environment by its supervisor.
const CHILD_ENV: &str = "RBXTS_WATCH_CHILD";
/// The generation the child's first rebuild continues from.
const GENERATION_ENV: &str = "RBXTS_WATCH_GENERATION";
/// Where a child that asks for a restart leaves its last generation.
const GENERATION_FILE_ENV: &str = "RBXTS_WATCH_GENERATION_FILE";
const DEBOUNCE: Duration = Duration::from_millis(100);
/// `RBXTS_WATCH_DEBUG` set: the watched directories and every raw event go to stderr.
fn debug() -> bool {
    static DEBUG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DEBUG.get_or_init(|| std::env::var_os("RBXTS_WATCH_DEBUG").is_some())
}

/// The touched path standing for "events were lost".
const RESCAN: &str = "<rescan>";

/// The time as `hh:mm:ss AM` (UTC: Rust's standard library has no time zone; tsrs's watch status does the same on
/// Windows).
fn time_now() -> String {
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()) % 86_400;
    let (hour, min, sec) = (secs / 3600, secs / 60 % 60, secs % 60);
    let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
    format!("{hour12:02}:{min:02}:{sec:02} {}", if hour < 12 { "AM" } else { "PM" })
}

/// A watch status line, as TypeScript's pretty watch status reporter prints it.
fn status(message: &str) {
    println!("[\u{1b}[90m{}\u{1b}[0m] {message}", time_now());
}

fn json_string(text: &str) -> String {
    serde_json::Value::String(text.to_owned()).to_string()
}

fn json_list(items: &[String]) -> String {
    serde_json::Value::Array(items.iter().map(|s| serde_json::Value::String(s.clone())).collect()).to_string()
}

/// The number of error diagnostics and fatal messages of a build.
pub fn error_count(report: &CompileReport) -> usize {
    let mut errors = report.diagnostics.iter().filter(|d| d.category() == Category::Error).count();
    errors += report.files.iter().flat_map(|f| &f.diagnostics).filter(|d| d.category() == Category::Error).count();
    errors + report.fatal.len()
}

/// Runs watch mode: as the supervisor (which restarts the watcher past the memory limit) unless this process is the
/// watcher child. Returns the exit status.
pub fn run(
    plugins: &[Box<dyn rbxts_plugin::erased::DynPlugin>],
    build: &BuildArgs,
    options: CompileOptions,
    cwd: &str,
) -> u8 {
    if std::env::var(CHILD_ENV).is_err() && memory_limit().is_some() {
        return supervise();
    }
    if std::env::var(CHILD_ENV).is_ok() {
        // the supervisor holds this process's stdin open: end-of-file means it is gone
        std::thread::spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            std::process::exit(0);
        });
    }
    let generation = std::env::var(GENERATION_ENV).ok().and_then(|g| g.parse().ok()).unwrap_or(0);
    watch(plugins, build, options, cwd, generation)
}

/// The working-set limit in bytes, `None` when turned off.
fn memory_limit() -> Option<usize> {
    let mb = std::env::var("RBXTS_WATCH_MEMORY_LIMIT_MB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(2048);
    (mb > 0).then_some(mb << 20)
}

/// The supervisor: runs the watcher as a child process with the same arguments and starts a new one whenever it
/// asks for a restart.
fn supervise() -> u8 {
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("rbxtsc: cannot find its own executable to run the watcher");
        return 1;
    };
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mut generation: u64 = 0;
    let generation_file = std::env::temp_dir().join(format!("rbxtsc-watch-{}.generation", std::process::id()));
    loop {
        // the child's stdin is a pipe this process holds open: when the supervisor goes (killed, not only by
        // Ctrl+C, which reaches both), the child reads end-of-file and exits too
        let child = std::process::Command::new(&exe)
            .args(&args)
            .env(CHILD_ENV, "1")
            .env(GENERATION_ENV, generation.to_string())
            .env(GENERATION_FILE_ENV, &generation_file)
            .stdin(std::process::Stdio::piped())
            .spawn();
        let status = child.and_then(|mut child| {
            let stdin = child.stdin.take();
            let status = child.wait();
            drop(stdin);
            status
        });
        let last = std::fs::read_to_string(&generation_file).ok().and_then(|g| g.trim().parse().ok());
        let _ = std::fs::remove_file(&generation_file);
        match status {
            Ok(status) if status.code() == Some(RESTART_EXIT_CODE) => {
                generation = last.unwrap_or(generation);
                status_line_restart();
            }
            Ok(status) => return u8::try_from(status.code().unwrap_or(1)).unwrap_or(1),
            Err(e) => {
                eprintln!("rbxtsc: could not start the watcher: {e}");
                return 1;
            }
        }
    }
}

fn status_line_restart() {
    status("The watcher reached its memory limit; restarting it (RBXTS_WATCH_MEMORY_LIMIT_MB).");
}

/// What the watcher watches, and which events count.
#[derive(Clone, Default)]
struct Watched {
    /// Path keys of the tsconfig, the configs it extends, the Rojo project files and the plugins' input files: their
    /// events always count, whatever directory they are in (an outDir set to the project directory included).
    config_inputs: FxHashSet<String>,
    recursive: BTreeSet<String>,
    flat: BTreeSet<String>,
    /// Path keys of the directories whose events are dropped (output roots, include folder).
    ignored_roots: Vec<String>,
    /// Path keys of the recursive roots (source roots, plugin `watch_dir`s): any event under them counts (but
    /// under a `node_modules` or `.git` in them).
    root_keys: Vec<String>,
    /// Path keys of the directories whose entries count when the program could read them (the directories of
    /// program files outside packages, the directories the tsconfig's include globs listed; the project directory
    /// before any project loaded): a directory, or a file the tsconfig's `files` and `include` could take in
    /// (every file while `include` is not known).
    open_dirs: FxHashSet<String>,
    /// The project directory's path key: its Rojo project files count (the project's Rojo config is looked for
    /// there), its other entries as any directory's (lockfiles and the like do not).
    project_dir: String,
    /// Path keys of the folders the Rojo resolvers search for nested project files (`$path` folders outside the
    /// ignored roots, the type roots): a project file, or a folder holding one, created or removed there counts.
    rojo_dirs: Vec<String>,
    /// Which files the program could take in, for `open_dirs`.
    matcher: Option<IncludeMatcher>,
    /// Path keys of the JSON files the last load read, with the hash of their text: an event leaving the same text
    /// does not count.
    texts: FxHashMap<String, u64>,
    /// Path keys of `node_modules`, its scope directories and the type roots: the directory itself, and any entry
    /// added, removed or renamed in it (but dot entries) counts, whether it exists yet or not.
    module_dirs: FxHashSet<String>,
    /// Path keys of package directories, watched recursively: a `package.json` or other JSON file (Rojo project
    /// files), a TypeScript or Luau file or a directory anywhere in them counts when the package is used.
    package_dirs: FxHashSet<String>,
    /// Path keys of the packages the build uses (a program file or a read of the load in them, a plugin resolution
    /// through them, an unresolved import naming them). The other packages are in a type root only: setup reads
    /// their `package.json` (an anchor) and the package resolver searches them for project files (`rojo_dirs`);
    /// nothing else in them counts.
    used_packages: FxHashSet<String>,
    /// Path keys of the package directories plugin resolutions go through: any event counts.
    plugin_package_dirs: FxHashSet<String>,
    /// Path keys whose own events count: every path the last load read, the paths module resolution probed that did
    /// not exist included, and the missing directories on the way to them.
    anchors: FxHashSet<String>,
    /// What the last load read (path, what it learned), for comparing a directory watched for the first time.
    reads: Vec<(String, Seen)>,
    /// The program's files with the modification time and size they were parsed at.
    stamps: Vec<(String, Option<SystemTime>, i64)>,
    /// The plugins' recorded inputs.
    inputs: inputs::InputWatches,
}

/// The package root of a path under `node_modules`: `…/node_modules/name` or `…/node_modules/@scope/name`.
fn package_root(path: &str) -> Option<String> {
    if !path.contains("node_modules") {
        return None;
    }
    let normalized = nodepath::normalize(path).replace('\\', "/");
    let at = normalized.rfind("/node_modules/")? + "/node_modules/".len();
    let rest = &normalized[at..];
    let mut parts = rest.split('/');
    let first = parts.next().filter(|f| !f.is_empty())?;
    let length = if first.starts_with('@') {
        first.len() + 1 + parts.next().filter(|p| !p.is_empty())?.len()
    } else {
        first.len()
    };
    Some(normalized[..at + length].to_owned())
}

fn key(path: &str) -> String {
    // TypeScript's spellings (most of the read log) are normalized already
    let plain = !path.contains('\\') && !path.contains("/.") && !path.contains("//") && !path.ends_with('/');
    if plain { path.to_lowercase() } else { nodepath::normalize(path).replace('\\', "/").to_lowercase() }
}

fn is_under(path_key: &str, root_key: &str) -> bool {
    path_key == root_key
        || (path_key.len() > root_key.len()
            && path_key.starts_with(root_key)
            && (root_key.ends_with('/') || path_key.as_bytes()[root_key.len()] == b'/'))
}

/// The parent of a path key or path, `None` at a root.
fn parent_of(path: &str) -> Option<&str> {
    let parent = nodepath::dirname(path);
    (parent != path && !parent.is_empty()).then_some(parent)
}

fn is_dir(path: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_dir())
}

/// Whether an entry of a package counts: package.json and other JSON (Rojo project files, JSON modules), TypeScript
/// and Luau files, directories (one created, moved in or removed: on Windows a directory moved in is one event).
fn package_entry_counts(path: &str, name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }
    const EXTENSIONS: [&str; 9] = [".json", ".ts", ".tsx", ".mts", ".cts", ".lua", ".luau", ".toml", ".d.ts"];
    if EXTENSIONS.iter().any(|e| name.ends_with(e)) {
        return true;
    }
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.is_dir(),
        // gone: a directory removed or renamed away (its name has no extension) counts
        Err(_) => !name.contains('.'),
    }
}

/// Whether a folder holds a Rojo project file anywhere below it. A link counts (the resolvers follow links; this
/// does not).
fn holds_project_file(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    entries.flatten().any(|entry| match entry.file_type() {
        Ok(t) if t.is_symlink() => true,
        Ok(t) if t.is_dir() => holds_project_file(&entry.path()),
        Ok(_) => rojo::is_rojo_project_file_name(&entry.file_name().to_string_lossy()),
        Err(_) => true,
    })
}

/// Whether the text of `path` now is the one a load read (`hash`, [`text_hash`]).
fn same_text(path: &str, hash: u64) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .is_some_and(|text| text_hash(&text) == hash)
}

impl Watched {
    /// Whether an entry of a folder a Rojo resolver searches changes what it finds: a project file, a folder holding
    /// one (created or moved in; a link counts), or a folder a project file the resolvers read was in (removed or
    /// moved away).
    fn rojo_entry_counts(&self, path: &str, path_key: &str, name: &str) -> bool {
        if rojo::is_rojo_project_file_name(name) {
            return true;
        }
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.file_type().is_symlink() => true,
            Ok(m) if m.is_dir() => holds_project_file(Path::new(path)),
            Ok(_) => false,
            Err(_) => self.config_inputs.iter().any(|input| is_under(input, path_key)),
        }
    }

    /// Whether an entry of an open directory (not under a root) counts: a directory, or a file the program could
    /// take in.
    fn open_entry_counts(&self, path: &str, name: &str) -> bool {
        let includable = || self.matcher.is_none_or(|m| m.possibly_includes(path));
        match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => true,
            Ok(_) => includable(),
            // gone: a directory (no extension) or a file the program could have taken in
            Err(_) => !name.contains('.') || includable(),
        }
    }

    /// Watches `path`'s deepest existing directory (its own directory when it exists, else the nearest existing
    /// ancestor), and makes the path and every missing directory on the way to it count: a file or directory
    /// appearing there (or going) changes what a load reads.
    fn cover(&mut self, path: &str, dirs: &mut FxHashMap<String, bool>) {
        let mut exists = |p: &str| *dirs.entry(key(p)).or_insert_with(|| is_dir(p));
        self.anchors.insert(key(path));
        let mut dir = match parent_of(path) {
            Some(parent) => parent.to_owned(),
            None => return,
        };
        // the common case: the directory was watched already for another path
        if self.flat.contains(&dir) {
            return;
        }
        while !exists(&dir) {
            self.anchors.insert(key(&dir));
            match parent_of(&dir) {
                Some(parent) => dir = parent.to_owned(),
                None => return,
            }
        }
        self.flat.insert(dir);
    }

    /// Adds what a load read: each path counts and is covered; directories the tsconfig's globs or the program
    /// listed are watched, their entries counting; the packages read from are package directories, used unless
    /// only their `package.json` was read (setup's package mapping reads every type-root package's); the JSON
    /// texts read.
    fn add_log(&mut self, log: &ReadLog, dirs: &mut FxHashMap<String, bool>) {
        self.texts.extend(log.texts().into_iter().map(|(path, hash)| (key(&path), hash)));
        let reads = log.entries();
        // the directories the load found (a directory, or a file's directory) need no look of their own; one gone
        // since is caught when its watch is made (`Subscriptions::update`)
        for (path, seen) in &reads {
            if seen.has(Seen::DIR) {
                dirs.insert(key(path), true);
            }
            if seen.has(Seen::FILE)
                && let Some(parent) = parent_of(path)
            {
                dirs.insert(key(parent), true);
            }
        }
        for (path, seen) in &reads {
            let k = key(path);
            if self.ignored_roots.iter().any(|root| is_under(&k, root)) {
                continue;
            }
            self.cover(path, dirs);
            if seen.has(Seen::LISTED) && *dirs.entry(k.clone()).or_insert_with(|| is_dir(path)) {
                self.flat.insert(path.clone());
                if k.split('/').any(|c| c == "node_modules") {
                    self.module_dirs.insert(k.clone());
                } else {
                    self.open_dirs.insert(k.clone());
                }
            }
            if let Some(root) = package_root(path) {
                let root_key = key(&root);
                let used = k != root_key && k != format!("{root_key}/package.json");
                self.add_package(&root, dirs, used);
            }
        }
        self.reads.extend(reads);
    }

    /// A package directory: watched recursively when it exists; it and its contents count (see `used_packages`).
    fn add_package(&mut self, dir: &str, dirs: &mut FxHashMap<String, bool>, used: bool) {
        let k = key(dir);
        if used {
            self.used_packages.insert(k.clone());
        }
        if self.package_dirs.insert(k.clone()) {
            let exists = *dirs.entry(k.clone()).or_insert_with(|| is_dir(dir));
            self.anchors.insert(k);
            if exists {
                self.recursive.insert(dir.to_owned());
            }
        }
    }

    /// The folders the Rojo resolvers search (`rojo_dirs`): watched recursively when they exist (unless a module
    /// directory, whose packages are watched already, or inside a recursive watch), and covered (a `$path` folder
    /// appearing or going).
    fn add_rojo_dirs(&mut self, partitions: Vec<String>, dirs: &mut FxHashMap<String, bool>) {
        let recursive: Vec<String> = self.recursive.iter().map(|d| key(d)).collect();
        for partition in partitions {
            let k = key(&partition);
            if self.ignored_roots.iter().any(|root| is_under(&k, root)) || self.rojo_dirs.contains(&k) {
                continue;
            }
            self.rojo_dirs.push(k.clone());
            if self.module_dirs.contains(&k) {
                continue;
            }
            self.cover(&partition, dirs);
            if !recursive.iter().any(|r| is_under(&k, r)) && *dirs.entry(k).or_insert_with(|| is_dir(&partition)) {
                self.recursive.insert(partition);
            }
        }
    }

    /// The package directory a path key is in, if any.
    fn package_of(&self, path_key: &str) -> Option<&str> {
        let mut dir = path_key;
        while let Some(parent) = parent_of(dir) {
            if let Some(package) = self.package_dirs.get(parent) {
                return Some(package);
            }
            dir = parent;
        }
        None
    }

    /// A `node_modules` or scope directory: it and its entries count, watched when it exists, its parent watched
    /// (or the nearest existing ancestor) so it appearing or going is seen.
    fn add_module_dir(&mut self, dir: &str, dirs: &mut FxHashMap<String, bool>) {
        self.module_dirs.insert(key(dir));
        self.cover(dir, dirs);
        if is_dir(dir) {
            self.flat.insert(dir.to_owned());
        }
    }

    /// Whether a path key is in a package directory.
    fn in_package(&self, path_key: &str) -> bool {
        self.package_of(path_key).is_some()
    }

    /// One watch per directory, whatever its spelling; none for a directory inside a recursive watch.
    fn dedupe(&mut self) {
        let mut seen = FxHashSet::default();
        self.recursive.retain(|dir| seen.insert(key(dir)));
        let recursive_keys = seen;
        let mut seen = FxHashSet::default();
        self.flat.retain(|dir| {
            let k = key(dir);
            let mut ancestor = Some(k.as_str());
            while let Some(a) = ancestor {
                if recursive_keys.contains(a) {
                    return false;
                }
                ancestor = parent_of(a);
            }
            seen.insert(k)
        });
    }
}

/// The tsconfig's directory, watched before any project could be loaded (a broken tsconfig at the first build: any
/// entry counts), and the `node_modules` beside it, which counts even when it does not exist yet (a fresh clone, then
/// an install).
fn minimal_set(session: &BuildSession<'_>) -> Watched {
    let mut watched = Watched::default();
    let tsconfig = nodepath::resolve(&[&session.options.cwd, &session.options.tsconfig]);
    let project_dir = nodepath::dirname(&tsconfig).to_owned();
    watched.flat.insert(project_dir.clone());
    watched.open_dirs.insert(key(&project_dir));
    watched.project_dir = key(&project_dir);
    watched.config_inputs.insert(key(&tsconfig));
    let mut dirs = FxHashMap::default();
    watched.add_module_dir(&nodepath::join(&[&project_dir, "node_modules"]), &mut dirs);
    watched
}

/// What to watch for the last loaded project; `None` when the last build loaded none (then the last good set is
/// kept: a failing load says nothing about where the sources are).
fn watched_set(session: &BuildSession<'_>) -> Option<Watched> {
    let mut watched = minimal_set(session);
    let mut dirs: FxHashMap<String, bool> = FxHashMap::default();
    let (services, program_files, root_dirs) = session.watch_roots()?;
    let project_dir = services.data.project_path.clone();
    // a project loaded: the project directory's entries count as the build could read them
    watched.open_dirs.remove(&watched.project_dir);
    watched.project_dir = key(&project_dir);
    watched.matcher = session.include_matcher();
    let out_dir = session.options.out_dir_override.clone().unwrap_or_else(|| services.path_translator.out_dir.clone());
    // an output root holding the project, a source root or the tsconfig would drop the events that matter (the
    // build refuses such a root anyway): it is never ignored
    let protected: Vec<String> = root_dirs
        .iter()
        .map(|r| key(r))
        .chain([key(&project_dir), key(&nodepath::resolve(&[&session.options.cwd, &session.options.tsconfig]))])
        .collect();
    for root in
        [out_dir, services.path_translator.out_dir.clone(), nodepath::resolve(&[&services.data.options.include_path])]
    {
        let root = key(&root);
        if !protected.iter().any(|p| is_under(p, &root)) {
            watched.ignored_roots.push(root);
        }
    }
    for input in session.config_inputs() {
        watched.config_inputs.insert(key(&input));
        watched.cover(&input, &mut dirs);
    }
    for root in root_dirs.iter().map(|r| nodepath::resolve(&[r])).chain(session.plugin_watch_dirs()) {
        watched.root_keys.push(key(&root));
        watched.recursive.insert(root);
    }
    watched.flat.insert(project_dir.clone());
    // node_modules, its scopes and the type roots: packages appearing or going
    let node_modules = services.data.node_modules_path.clone();
    watched.add_module_dir(&node_modules, &mut dirs);
    if let Ok(entries) = std::fs::read_dir(&node_modules) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('@') && std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir()) {
                watched.add_module_dir(&nodepath::join(&[&node_modules, &name]), &mut dirs);
            }
        }
    }
    // every package in a type root is read by the setup (its package.json for the path mapping, its folders for
    // nested Rojo projects) and, without `types`, loaded by TypeScript
    for type_root in &services.type_roots {
        watched.add_module_dir(type_root, &mut dirs);
        if let Ok(entries) = std::fs::read_dir(type_root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with('.') && std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir()) {
                    watched.add_package(&nodepath::join(&[type_root, &name]), &mut dirs, false);
                }
            }
        }
    }
    // plugin module resolutions are build inputs: the package directories they go through count whole
    for (specifier, _from, resolved) in session.plugin_resolutions() {
        if let Some(resolved) = &resolved {
            let dir = nodepath::dirname(resolved).to_owned();
            watched.plugin_package_dirs.insert(key(&dir));
            watched.cover(resolved, &mut dirs);
            if let Some(root) = package_root(resolved) {
                watched.add_package(&root, &mut dirs, true);
            }
        }
        if !specifier.starts_with('.') && !nodepath::is_absolute(&specifier) {
            let mut package = node_modules.clone();
            for part in specifier.split('/').take(if specifier.starts_with('@') { 2 } else { 1 }) {
                package = nodepath::join(&[&package, part]);
                watched.plugin_package_dirs.insert(key(&package));
                watched.cover(&package, &mut dirs);
            }
            watched.add_package(&package, &mut dirs, true);
        }
    }
    // the packages an unresolved import names, where they would be installed
    for name in session.unresolved_packages() {
        let mut dir = node_modules.clone();
        for part in name.split('/').take(if name.starts_with('@') { 2 } else { 1 }) {
            dir = nodepath::join(&[&dir, part]);
        }
        watched.cover(&dir, &mut dirs);
        watched.add_package(&dir, &mut dirs, true);
    }
    for file in &program_files {
        if file.starts_with("bundled:") {
            continue;
        }
        if let Some(root) = package_root(file) {
            watched.add_package(&root, &mut dirs, true);
            continue;
        }
        let dir = nodepath::dirname(file).to_owned();
        watched.open_dirs.insert(key(&dir));
        watched.flat.insert(dir);
    }
    if let Some(log) = session.read_log() {
        watched.add_log(&log, &mut dirs);
    }
    watched.add_rojo_dirs(session.rojo_partitions(), &mut dirs);
    watched.stamps = session.loaded_stamps();
    watched.inputs = inputs::InputWatches::of(session);
    watched.recursive.extend(watched.inputs.recursive.iter().cloned());
    watched.flat.extend(watched.inputs.flat.iter().cloned());
    watched.dedupe();
    Some(watched)
}

/// Whether an event path matters. `folder_modify`: the event is only a modification of an existing directory (its
/// entries changed), which counts only for a plugin package directory (the events of the entries say more).
fn relevant(path: &str, folder_modify: bool, watched: &Watched, own: &FxHashMap<String, Version>) -> bool {
    let k = key(path);
    // a JSON file the load read, with the same text again (a package manager rewriting package.json as it was)
    if let Some(hash) = watched.texts.get(&k)
        && same_text(path, *hash)
    {
        return false;
    }
    // configs and build inputs first: roblox-ts's watcher checks isConfigPath before its output-path ignore
    if watched.config_inputs.contains(&k) {
        return true;
    }
    if watched.ignored_roots.iter().any(|root| is_under(&k, root)) {
        return false;
    }
    if folder_modify {
        return watched.plugin_package_dirs.contains(&k);
    }
    let parent = nodepath::dirname(&k);
    let name = nodepath::basename(&k);
    if name.starts_with('.') && name.contains(".rbxts-tmp-") {
        return false;
    }
    // this watcher's own artifacts and state files, as it wrote them
    if let Some(version) = own.get(&k)
        && std::fs::read(path).is_ok_and(|bytes| Version::of(&String::from_utf8_lossy(&bytes)) == *version)
    {
        return false;
    }
    // package names never start with a dot: `node_modules/.package-lock.json`, `.cache`, `.bin` are churn
    let in_module_dir = watched.module_dirs.contains(parent);
    if in_module_dir {
        return !name.starts_with('.');
    }
    if watched.anchors.contains(&k) || watched.module_dirs.contains(&k) {
        return true;
    }
    if watched.plugin_package_dirs.contains(parent) || watched.plugin_package_dirs.contains(&k) {
        return true;
    }
    // the project's Rojo config is looked for in the project directory
    if parent == watched.project_dir && (rojo::is_rojo_project_file_name(name) || name == "roblox-project.json") {
        return true;
    }
    if watched.rojo_dirs.iter().any(|dir| is_under(parent, dir)) && watched.rojo_entry_counts(path, &k, name) {
        return true;
    }
    if let Some(package) = watched.package_of(&k) {
        return watched.used_packages.contains(package) && package_entry_counts(path, name);
    }
    if k.split('/').any(|c| c == "node_modules" || c == ".git") {
        return false;
    }
    if watched.root_keys.iter().any(|root| is_under(&k, root)) {
        return true;
    }
    watched.open_dirs.contains(parent) && watched.open_entry_counts(path, name)
}

/// The watches of the last build that loaded a project, and those in use after a build that loaded none.
struct Watches {
    good: Watched,
    failed: Option<Watched>,
}

impl Watches {
    fn current(&self) -> &Watched {
        self.failed.as_ref().unwrap_or(&self.good)
    }
}

/// A watched directory and whether its watch is recursive.
type Watch = (String, bool);

/// An event path of a burst, flagged when all its events were modifications of an existing directory.
type BurstPath = (String, bool);

struct Subscriptions {
    watcher: Box<dyn Watcher>,
    /// The watches made.
    active: FxHashSet<(String, bool)>,
    /// The wanted directories that were not there at the last update.
    missing: FxHashSet<String>,
    /// The watches that failed, reported once; tried again at each update until one succeeds.
    failed: FxHashSet<(String, bool)>,
}

impl Subscriptions {
    /// Watches what `watched` lists. Returns the paths in the directories it starts watching that changed since
    /// `since` (the build's start) and matter: a write between the program load and the new watch (a package
    /// folder filled during the build that its creation started, a `watch_dir` root) raised no event here, so
    /// those paths start another rebuild.
    fn update(&mut self, watched: &Watched, since: SystemTime, own: &FxHashMap<String, Version>) -> Vec<String> {
        if debug() {
            eprintln!("[rbxtsc watch] recursive {:?}\n[rbxtsc watch] flat {:?}", watched.recursive, watched.flat);
        }
        let (wanted, missing): (FxHashSet<Watch>, FxHashSet<Watch>) = watched
            .recursive
            .iter()
            .map(|d| (d.clone(), true))
            .chain(watched.flat.iter().map(|d| (d.clone(), false)))
            .partition(|(d, _)| Path::new(d).is_dir());
        // a directory the build found that is gone already (removed in between: its removal raised no event here)
        // starts a rebuild, once
        let missing: FxHashSet<String> = missing.into_iter().map(|(d, _)| d).collect();
        let mut stale: Vec<String> = missing.iter().filter(|d| !self.missing.contains(*d)).cloned().collect();
        self.missing = missing;
        for (dir, _) in self.active.iter().filter(|w| !wanted.contains(w)) {
            let _ = self.watcher.unwatch(Path::new(dir));
        }
        let new: Vec<&(String, bool)> = wanted.iter().filter(|w| !self.active.contains(w)).collect();
        let mut active: FxHashSet<Watch> = self.active.intersection(&wanted).cloned().collect();
        if !new.is_empty() {
            let scan = Scan::new(watched, since, own);
            for watch @ (dir, recursive) in new {
                let mode = if *recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
                match self.watcher.watch(Path::new(dir), mode) {
                    Ok(()) => {
                        self.failed.remove(watch);
                        active.insert(watch.clone());
                    }
                    // not active: tried again at the next update (a failure is reported once)
                    Err(e) => {
                        if self.failed.insert(watch.clone()) {
                            eprintln!("rbxtsc: cannot watch {dir}: {e}");
                        }
                    }
                }
                let whole = *recursive || watched.module_dirs.contains(&key(dir));
                scan.dir(Path::new(dir), *recursive, whole, &mut stale);
            }
        }
        self.failed.retain(|w| wanted.contains(w));
        self.active = active;
        stale.sort();
        stale.dedup();
        if debug() && !stale.is_empty() {
            eprintln!("[rbxtsc watch] changed before their directory was watched: {stale:?}");
        }
        stale
    }
}

/// A program file, with the modification time and size it was parsed at.
type Stamp<'a> = (&'a str, Option<SystemTime>, i64);

/// Compares a directory watched for the first time with what the build read: what changed there since the build
/// started raised no event (the watch did not exist yet).
struct Scan<'a> {
    watched: &'a Watched,
    since: SystemTime,
    own: &'a FxHashMap<String, Version>,
    /// What the load read, by the path key of the directory holding it.
    reads: FxHashMap<String, Vec<(&'a str, Seen)>>,
    /// The program's files as parsed, by the path key of their directory.
    stamps: FxHashMap<String, Vec<Stamp<'a>>>,
    /// Path keys of the directories this watcher wrote into (their modification time is its own).
    own_dirs: FxHashSet<String>,
}

impl<'a> Scan<'a> {
    fn new(watched: &'a Watched, since: SystemTime, own: &'a FxHashMap<String, Version>) -> Scan<'a> {
        let mut reads: FxHashMap<String, Vec<(&str, Seen)>> = FxHashMap::default();
        for (path, seen) in &watched.reads {
            if let Some(parent) = parent_of(path) {
                reads.entry(key(parent)).or_default().push((path, *seen));
            }
        }
        let mut stamps: FxHashMap<String, Vec<Stamp<'_>>> = FxHashMap::default();
        for (path, time, size) in &watched.stamps {
            if let Some(parent) = parent_of(path) {
                stamps.entry(key(parent)).or_default().push((path, *time, *size));
            }
        }
        let own_dirs = own.keys().filter_map(|k| parent_of(k).map(str::to_owned)).collect();
        Scan { watched, since, own, reads, stamps, own_dirs }
    }

    /// A file written or created since the build started, or a directory created since (a directory's modification
    /// time is its entries': those are judged on their own, or by the directory's own scan).
    fn recent(&self, metadata: &std::fs::Metadata) -> bool {
        metadata.created().is_ok_and(|t| t >= self.since)
            || (!metadata.is_dir() && metadata.modified().is_ok_and(|t| t >= self.since))
    }

    /// Adds to `stale` what in `dir` (and below it when `recursive`) differs from what the build read or changed
    /// since it started. `whole`: any entry of the directory counts (a module, package or root directory), so its
    /// own modification time counts too (an entry added, removed or renamed, whatever the entry's times).
    fn dir(&self, dir: &Path, recursive: bool, whole: bool, stale: &mut Vec<String>) {
        let text = path_text(dir);
        let k = key(&text);
        if self.watched.ignored_roots.iter().any(|root| is_under(&k, root)) {
            return;
        }
        let Ok(metadata) = std::fs::metadata(dir) else { return };
        // the directory itself is new since the build started
        if metadata.created().is_ok_and(|t| t >= self.since) {
            stale.push(text);
            return;
        }
        if whole && metadata.modified().is_ok_and(|t| t >= self.since) && !self.own_dirs.contains(&k) {
            stale.push(text.clone());
        }
        // what the load read here: a file or directory there or not, as it was
        for (path, seen) in self.reads.get(&k).into_iter().flatten() {
            let metadata = std::fs::metadata(path);
            let (is_file, is_dir) = metadata.map_or((false, false), |m| (m.is_file(), m.is_dir()));
            if seen.contradicted(is_file, is_dir) {
                stale.push((*path).to_owned());
            }
        }
        // the program's files: the time and size they were parsed at (a file replaced with an old time)
        for (path, time, size) in self.stamps.get(&k).into_iter().flatten() {
            let now = std::fs::metadata(path).ok().map(|m| (m.modified().ok(), m.len() as i64));
            if now != Some((*time, *size)) {
                stale.push((*path).to_owned());
            }
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else { continue };
            let path = entry.path();
            let path_string = path_text(&path);
            if self.recent(&metadata) && relevant(&path_string, false, self.watched, self.own) {
                stale.push(path_string.clone());
            }
            let name = entry.file_name();
            // links are not followed into; a source root's node_modules is not a package (packages are scanned
            // as their own watches)
            if recursive
                && metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && name != ".git"
                && (name != "node_modules" || self.watched.in_package(&key(&path_string)))
            {
                self.dir(&path, true, whole, stale);
            }
        }
    }
}

fn watch(
    plugins: &[Box<dyn rbxts_plugin::erased::DynPlugin>],
    build: &BuildArgs,
    options: CompileOptions,
    cwd: &str,
    first_generation: u64,
) -> u8 {
    let plugin_options = PluginOptions { plugins, ..PluginOptions::default() };
    let mut session = BuildSession::new(options, &plugin_options, true);
    let (sender, receiver) = channel::<notify::Result<Event>>();
    let watcher: Box<dyn Watcher> = if build.use_polling {
        match notify::PollWatcher::new(sender, notify::Config::default().with_poll_interval(Duration::from_millis(250)))
        {
            Ok(w) => Box::new(w),
            Err(e) => {
                eprintln!("rbxtsc: cannot start the file watcher: {e}");
                return 1;
            }
        }
    } else {
        match notify::recommended_watcher(sender) {
            Ok(w) => Box::new(w),
            Err(e) => {
                eprintln!("rbxtsc: cannot start the file watcher: {e}");
                return 1;
            }
        }
    };
    let mut subscriptions = Subscriptions {
        watcher,
        active: FxHashSet::default(),
        missing: FxHashSet::default(),
        failed: FxHashSet::default(),
    };
    let mut own: FxHashMap<String, Version> = FxHashMap::default();
    // a restarted watcher goes on from the last generation of the one before
    let mut generation = if first_generation > 0 { first_generation + 1 } else { 0 };

    let mut watched = Watches { good: minimal_set(&session), failed: None };
    status("Starting compilation in watch mode...");
    let mut stale = rebuild(&mut session, &mut subscriptions, &mut watched, build, cwd, generation, &[], &mut own);
    loop {
        // paths that changed before their directory was watched (already judged) start a rebuild of their own
        let mut paths: Vec<String> = if stale.is_empty() {
            let Some(events) = collect(&receiver) else { return 0 };
            // a directory watched from now on is compared with the disk from here (the answers below read it)
            let since = SystemTime::now();
            let (relevant, dropped): (Vec<BurstPath>, Vec<BurstPath>) = events
                .into_iter()
                .partition(|(p, folder_modify)| p == RESCAN || relevant(p, *folder_modify, watched.current(), &own));
            let mut paths: Vec<String> = relevant.into_iter().map(|(p, _)| p).collect();
            // what only a plugin's recorded input may care about: with a rebuild anyway, or when an answer changed
            let candidates = inputs::candidates(dropped, &watched.current().inputs, &own);
            if !candidates.is_empty() {
                if !paths.is_empty() || session.recorded_inputs_changed(Some(&candidates)) {
                    paths.extend(candidates);
                } else {
                    stale = inputs::refresh(&session, &mut subscriptions, &mut watched, &own, since);
                }
            }
            paths
        } else {
            std::mem::take(&mut stale)
        };
        paths.sort();
        paths.dedup();
        // a directory as a touched path makes the session load the project again, with every file's facts
        if let Some(rescan) = paths.iter_mut().find(|p| *p == RESCAN) {
            *rescan = session.options.tsconfig.clone(); // a config input: a rebuild with every file looked at
        }
        if paths.is_empty() {
            continue;
        }
        generation += 1;
        status("File change detected. Starting incremental compilation...");
        stale = rebuild(&mut session, &mut subscriptions, &mut watched, build, cwd, generation, &paths, &mut own);
        if let Some(limit) = memory_limit()
            && std::env::var(CHILD_ENV).is_ok()
            && memory_stats::memory_stats().is_some_and(|m| m.physical_mem > limit)
        {
            if let Ok(file) = std::env::var(GENERATION_FILE_ENV) {
                let _ = std::fs::write(file, generation.to_string());
            }
            std::process::exit(RESTART_EXIT_CODE);
        }
    }
}

/// Waits for an event, then for the burst to end (each event restarts the wait); the paths of the burst, sorted,
/// each flagged when all its events were modifications of an existing directory (see [`relevant`]). A directory
/// created, moved in or renamed is a plain event (on Windows a directory moved in is one event for the directory).
/// Whether a path is a directory is looked at once the burst ended, not while it runs: opening a directory being
/// deleted leaves it pending deletion while the handle is open, and the deleting tool (`npm`, `rm -rf`) failing
/// on it with EPERM. `None` when the watcher stopped.
fn collect(receiver: &Receiver<notify::Result<Event>>) -> Option<Vec<(String, bool)>> {
    let mut paths: BTreeMap<String, bool> = BTreeMap::new();
    let add = |event: notify::Result<Event>, paths: &mut BTreeMap<String, bool>| match event {
        _ if debug() && {
            eprintln!("[rbxtsc watch] event {event:?}");
            false
        } => {}
        // events were lost (an overflowing buffer): everything may have changed
        Err(_) => {
            paths.insert(RESCAN.to_owned(), false);
        }
        Ok(event) if event.need_rescan() => {
            paths.insert(RESCAN.to_owned(), false);
        }
        Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
            let modify = matches!(
                event.kind,
                EventKind::Modify(ModifyKind::Any | ModifyKind::Data(_) | ModifyKind::Metadata(_) | ModifyKind::Other)
            );
            // a path's flag: every event of it so far was a modification
            for path in event.paths {
                let entry = paths.entry(path_text(&path)).or_insert(modify);
                *entry &= modify;
            }
        }
        Ok(_) => {}
    };
    add(receiver.recv().ok()?, &mut paths);
    loop {
        match receiver.recv_timeout(DEBOUNCE) {
            Ok(event) => add(event, &mut paths),
            Err(RecvTimeoutError::Timeout) => {
                return Some(
                    paths.into_iter().map(|(p, modify)| (p.clone(), modify && Path::new(&p).is_dir())).collect(),
                );
            }
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

fn path_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

/// One build of the watch loop, with its report and events. A panic is reported, and the session starts over.
/// After the build, the watched directories are brought up to date before anything is printed, so a change made
/// as soon as the build is reported finished is seen.
#[allow(clippy::too_many_arguments)]
fn rebuild(
    session: &mut BuildSession<'_>,
    subscriptions: &mut Subscriptions,
    watched: &mut Watches,
    build: &BuildArgs,
    cwd: &str,
    generation: u64,
    paths: &[String],
    own: &mut FxHashMap<String, Version>,
) -> Vec<String> {
    if build.events_json {
        println!("{{\"event\":\"build-start\",\"gen\":{generation},\"changes\":{}}}", json_list(paths));
    }
    let since = SystemTime::now();
    let result = catch_unwind(AssertUnwindSafe(|| session.build(paths)));
    if result.is_err() {
        session.reset();
    }
    // this build's own writes are known before the new watches look for changes
    if let Ok((_, summary)) = &result {
        for path in &summary.written {
            if let Ok(bytes) = std::fs::read(path) {
                own.insert(key(path), Version::of(&String::from_utf8_lossy(&bytes)));
            }
        }
    }
    let start = std::time::Instant::now();
    match watched_set(session) {
        Some(set) => {
            watched.good = set;
            watched.failed = None;
        }
        // no project loaded: the last good set, and what the failed load read (a tsconfig extending a package not
        // installed yet)
        None => {
            let mut set = watched.good.clone();
            if let Some(log) = session.read_log() {
                set.add_log(&log, &mut FxHashMap::default());
                set.dedupe();
            }
            watched.failed = Some(set);
        }
    }
    let computed = start.elapsed();
    let new_input_watches = watched.current().inputs.has_new_watches(&subscriptions.active);
    let mut stale = subscriptions.update(watched.current(), since, own);
    // a recorded input read before its directory was watched: asked again now that it is
    if result.is_ok() {
        stale.extend(inputs::recheck(session, &watched.current().inputs, new_input_watches));
    }
    if debug() {
        let current = watched.current();
        eprintln!(
            "[rbxtsc watch] watch set {:.1} ms, update and scan {:.1} ms: {} reads, {} anchors, {} packages, {} recursive, {} flat",
            computed.as_secs_f64() * 1000.0,
            (start.elapsed() - computed).as_secs_f64() * 1000.0,
            current.reads.len(),
            current.anchors.len(),
            current.package_dirs.len(),
            current.recursive.len(),
            current.flat.len()
        );
    }
    let (report, summary) = match result {
        Ok(result) => result,
        Err(panic) => {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "panic".to_owned());
            eprintln!("\u{1b}[31mrbxtsc: the build crashed:\u{1b}[0m {message}\nThe next build starts clean.");
            if build.events_json {
                println!(
                    "{{\"event\":\"build-end\",\"gen\":{generation},\"ok\":false,\"errors\":1,\"crash\":{},\"written\":[],\"deleted\":[]}}",
                    json_string(&message)
                );
            }
            status("Found 1 error. Watching for file changes.");
            return stale;
        }
    };
    print_report(&report, cwd, build);
    let errors = error_count(&report);
    if build.events_json {
        print_end(generation, &report, &summary, errors);
    }
    status(&format!("Found {errors} error{}. Watching for file changes.", if errors == 1 { "" } else { "s" }));
    stale
}

fn print_end(generation: u64, report: &CompileReport, summary: &BuildSummary, errors: usize) {
    let ok = !report.has_errors();
    println!(
        "{{\"event\":\"build-end\",\"gen\":{generation},\"ok\":{ok},\"errors\":{errors},\"all\":{},\"transformed\":{},\"written\":{},\"deleted\":{}}}",
        summary.all,
        report.files.len(),
        json_list(&summary.written),
        json_list(&summary.deleted)
    );
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    use notify::RecursiveMode;
    use rustc_hash::{FxHashMap, FxHashSet};

    use super::{Subscriptions, Watched, is_under, package_entry_counts, package_root, path_text, relevant};

    /// A watcher whose first watch of each path fails.
    struct FailFirst(Arc<Mutex<Vec<PathBuf>>>);

    impl notify::Watcher for FailFirst {
        fn new<F: notify::EventHandler>(_: F, _: notify::Config) -> notify::Result<Self> {
            Ok(FailFirst(Arc::default()))
        }

        fn watch(&mut self, path: &Path, _: RecursiveMode) -> notify::Result<()> {
            let mut tried = self.0.lock().unwrap();
            let first = !tried.iter().any(|p| p == path);
            tried.push(path.to_path_buf());
            if first { Err(notify::Error::generic("refused")) } else { Ok(()) }
        }

        fn unwatch(&mut self, _: &Path) -> notify::Result<()> {
            Ok(())
        }

        fn kind() -> notify::WatcherKind {
            notify::WatcherKind::NullWatcher
        }
    }

    #[test]
    fn a_failed_watch_is_tried_again_at_the_next_update() {
        let dir = std::env::temp_dir().join(format!("rbxts-rs-watch-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tried = Arc::new(Mutex::new(Vec::new()));
        let mut subscriptions = Subscriptions {
            watcher: Box::new(FailFirst(Arc::clone(&tried))),
            active: FxHashSet::default(),
            missing: FxHashSet::default(),
            failed: FxHashSet::default(),
        };
        let mut watched = Watched::default();
        let watch = (path_text(&dir), false);
        watched.flat.insert(watch.0.clone());
        let own = FxHashMap::default();
        subscriptions.update(&watched, SystemTime::now(), &own);
        assert!(!subscriptions.active.contains(&watch), "a failed watch is not active");
        assert!(subscriptions.failed.contains(&watch));
        subscriptions.update(&watched, SystemTime::now(), &own);
        assert!(subscriptions.active.contains(&watch), "tried again");
        assert!(subscriptions.failed.is_empty());
        subscriptions.update(&watched, SystemTime::now(), &own);
        assert_eq!(tried.lock().unwrap().len(), 2, "an active watch is not made again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_roots() {
        assert_eq!(
            package_root("E:/p/node_modules/@rbxts/types/include/a.d.ts").as_deref(),
            Some("E:/p/node_modules/@rbxts/types")
        );
        assert_eq!(package_root("E:/p/node_modules/pkg/lib/x.d.ts").as_deref(), Some("E:/p/node_modules/pkg"));
        assert_eq!(package_root("E:/p/src/x.ts"), None);
        // module directories themselves are no packages
        assert_eq!(package_root("E:/p/node_modules/@rbxts"), None);
        assert_eq!(package_root("E:/p/node_modules/@rbxts/"), None);
        assert_eq!(package_root("E:/p/node_modules/a/node_modules"), Some("E:/p/node_modules/a".to_owned()));
    }

    #[test]
    fn under() {
        assert!(is_under("e:/p/src/a.ts", "e:/p/src"));
        assert!(!is_under("e:/p/srcx/a.ts", "e:/p/src"));
        assert!(is_under("e:/p", "e:/"));
    }

    #[test]
    fn package_entries() {
        for name in ["package.json", "default.project.json", "index.d.ts", "init.lua", "init.luau", "x.tsx"] {
            assert!(package_entry_counts(&format!("Z:/none/{name}"), name), "{name}");
        }
        for name in ["README.md", ".npmignore", "LICENSE.txt", "x.js"] {
            assert!(!package_entry_counts(&format!("Z:/none/{name}"), name), "{name}");
        }
        // gone, no extension: a directory removed or renamed away
        assert!(package_entry_counts("Z:/none/out", "out"));
    }

    #[test]
    fn relevance() {
        let mut w = Watched::default();
        w.module_dirs.insert("e:/p/node_modules".into());
        w.module_dirs.insert("e:/p/node_modules/@rbxts".into());
        w.package_dirs.insert("e:/p/node_modules/@rbxts/pkg".into());
        w.used_packages.insert("e:/p/node_modules/@rbxts/pkg".into());
        w.package_dirs.insert("e:/p/node_modules/@rbxts/typeonly".into());
        w.anchors.insert("e:/p/node_modules/@rbxts/typeonly/package.json".into());
        w.rojo_dirs.push("e:/p/node_modules/@rbxts".into());
        w.rojo_dirs.push("e:/p/vendor".into());
        w.config_inputs.insert("e:/p/vendor/a/default.project.json".into());
        w.project_dir = "e:/p".into();
        w.anchors.insert("e:/node_modules".into());
        w.root_keys.push("e:/p/src".into());
        w.open_dirs.insert("e:/p".into());
        w.ignored_roots.push("e:/p/out".into());
        let own = Default::default();
        let yes = |p: &str| relevant(p, false, &w, &own);
        // the module directories themselves, and packages appearing or going in them
        assert!(yes("E:/p/node_modules"));
        assert!(yes("E:/p/node_modules/@rbxts/newpkg"));
        assert!(yes("E:/p/node_modules/@new"));
        assert!(!yes("E:/p/node_modules/.package-lock.json"));
        // inside a package, by kind; elsewhere under node_modules nothing
        assert!(yes("E:/p/node_modules/@rbxts/pkg/out/index.d.ts"));
        assert!(!yes("E:/p/node_modules/@rbxts/pkg/README.md"));
        assert!(!yes("E:/p/node_modules/other/index.d.ts"));
        // a package in a type root only: its package.json and Rojo project files, not its code
        assert!(yes("E:/p/node_modules/@rbxts/typeonly/package.json"));
        assert!(yes("E:/p/node_modules/@rbxts/typeonly/src/default.project.json"));
        assert!(!yes("E:/p/node_modules/@rbxts/typeonly/src/index.d.ts"));
        assert!(!yes("E:/p/node_modules/@rbxts/typeonly/src/init.lua"));
        assert!(!yes("E:/p/node_modules/@rbxts/typeonly/src/gone"));
        // a Rojo-mounted folder outside the sources: project files, and a folder that held a project file read
        assert!(yes("E:/p/vendor/x.project.json"));
        assert!(yes("E:/p/vendor/a"));
        assert!(!yes("E:/p/vendor/b"));
        assert!(!yes("E:/p/vendor/a/x.lua"));
        // a probed location outside the project, and its neighbours
        assert!(yes("E:/node_modules"));
        assert!(!yes("E:/elsewhere.txt"));
        // roots, open directories, outputs
        assert!(yes("E:/p/src/a/b.ts"));
        assert!(yes("E:/p/package.json"));
        assert!(!yes("E:/p/out/a.luau"));
        // the project directory: its Rojo project files; not lockfiles (not open, nothing read them)
        w.open_dirs.remove("e:/p");
        let yes = |p: &str| relevant(p, false, &w, &own);
        assert!(yes("E:/p/other.project.json"));
        assert!(!yes("E:/p/bun.lock"));
        assert!(!yes("E:/p/package-lock.json"));
        assert!(!yes("E:/p/sub/x.ts"));
        // a directory's own modification never counts here
        assert!(!relevant("E:/p/node_modules/@rbxts/pkg", true, &w, &own));
    }
}
