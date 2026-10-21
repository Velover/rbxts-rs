//! Watch mode (`rbxtsc -w`, DESIGN §14 "Watcher"): one [`BuildSession`] rebuilt on file system events.
//!
//! - **Events.** A `notify` watcher (ReadDirectoryChangesW on Windows, or polling with `--usePolling`) on the root
//!   directories (recursive), the tsconfig's directory and the directories of the configs it extends, of the Rojo
//!   project files and of the plugins' input files (not recursive), the directories plugins asked to watch
//!   (`watch_dir`, recursive), and the directories of program files outside those. Under `node_modules` (never
//!   recursively): `node_modules` and its `@scope` directories, where an added or removed package counts; the
//!   root and the declaration directories of every package the program uses, of every package an unresolved import
//!   names, and of every package plugin `resolve_module` calls went through, where a changed `package.json` or
//!   declaration file counts. Events under the output roots, the include folder and `.git`, other events under
//!   `node_modules`, a directory's own modification events (its entries' events say more; a directory created or
//!   moved in counts), rbxts-rs's own temporary files, and writes of this watcher's own artifacts are dropped.
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

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, SystemTime};

use notify::event::ModifyKind;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_project::nodepath;
use rbxts_transform::build::graph::Version;
use rbxts_transform::{BuildSession, BuildSummary, CompileOptions, CompileReport, PluginOptions};
use tsrs_diagnostics::Category;

use crate::{BuildArgs, print_report};

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

/// What the watcher watches and ignores.
#[derive(Default)]
struct Watched {
    recursive: BTreeSet<String>,
    flat: BTreeSet<String>,
    /// Path keys of the directories whose events are dropped (output roots, include folder).
    ignored_roots: Vec<String>,
    /// Path keys of `node_modules` and its scope directories: any entry added, removed or renamed counts.
    module_dirs: FxHashSet<String>,
    /// Path keys of package directories: a changed `package.json` or TypeScript file counts.
    package_dirs: FxHashSet<String>,
    /// Path keys of the package directories plugin resolutions go through: any event counts.
    plugin_package_dirs: FxHashSet<String>,
}

/// The package root of a path under `node_modules`: `…/node_modules/name` or `…/node_modules/@scope/name`.
fn package_root(path: &str) -> Option<String> {
    let normalized = nodepath::normalize(path).replace('\\', "/");
    let at = normalized.rfind("/node_modules/")? + "/node_modules/".len();
    let rest = &normalized[at..];
    let mut parts = rest.split('/');
    let first = parts.next()?;
    let length = if first.starts_with('@') { first.len() + 1 + parts.next()?.len() } else { first.len() };
    Some(normalized[..at + length].to_owned())
}

fn key(path: &str) -> String {
    nodepath::normalize(path).replace('\\', "/").to_lowercase()
}

fn is_under(path_key: &str, root_key: &str) -> bool {
    path_key == root_key
        || (path_key.len() > root_key.len()
            && path_key.starts_with(root_key)
            && path_key.as_bytes()[root_key.len()] == b'/')
}

fn watched_set(session: &BuildSession<'_>) -> Watched {
    let mut watched = Watched::default();
    // the tsconfig's directory, also when no project could be loaded (a broken tsconfig at the first build)
    watched
        .flat
        .insert(nodepath::dirname(&nodepath::resolve(&[&session.options.cwd, &session.options.tsconfig])).to_owned());
    let Some((services, program_files, root_dirs)) = session.watch_roots() else { return watched };
    let project_dir = services.data.project_path.clone();
    let out_dir = session.options.out_dir_override.clone().unwrap_or_else(|| services.path_translator.out_dir.clone());
    watched.ignored_roots.push(key(&out_dir));
    watched.ignored_roots.push(key(&services.path_translator.out_dir));
    watched.ignored_roots.push(key(&nodepath::resolve(&[&services.data.options.include_path])));
    for root in &root_dirs {
        watched.recursive.insert(nodepath::resolve(&[root]));
    }
    for dir in session.plugin_watch_dirs() {
        watched.recursive.insert(dir);
    }
    watched.flat.insert(project_dir);
    for input in session.config_inputs() {
        watched.flat.insert(nodepath::dirname(&input).to_owned());
    }
    // plugin module resolutions are build inputs: watch the package directories they go through (not recursive:
    // a package added, removed or reinstalled, its package.json, its types entry), so a change rebuilds
    let node_modules = services.data.node_modules_path.clone();
    for (specifier, _from, resolved) in session.plugin_resolutions() {
        let mut dirs = Vec::new();
        if let Some(resolved) = &resolved {
            dirs.push(nodepath::dirname(resolved).to_owned());
        }
        if !specifier.starts_with('.') && !nodepath::is_absolute(&specifier) {
            dirs.push(node_modules.clone());
            let mut package = node_modules.clone();
            for part in specifier.split('/').take(if specifier.starts_with('@') { 2 } else { 1 }) {
                package = nodepath::join(&[&package, part]);
                dirs.push(package.clone());
            }
        }
        for dir in dirs {
            watched.plugin_package_dirs.insert(key(&dir));
            watched.flat.insert(dir);
        }
    }
    // packages: node_modules and its scopes for packages appearing or going, the packages in use (and those an
    // unresolved import names) for their package.json and declaration files
    let mut module_dirs = vec![node_modules.clone()];
    if let Ok(entries) = std::fs::read_dir(&node_modules) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('@') && std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir()) {
                module_dirs.push(nodepath::join(&[&node_modules, &name]));
            }
        }
    }
    for dir in module_dirs {
        watched.module_dirs.insert(key(&dir));
        watched.flat.insert(dir);
    }
    let mut package_dirs: BTreeSet<String> = BTreeSet::new();
    for name in session.unresolved_packages() {
        let package: Vec<&str> = name.split('/').take(if name.starts_with('@') { 2 } else { 1 }).collect();
        let mut dir = node_modules.clone();
        for part in package {
            dir = nodepath::join(&[&dir, part]);
        }
        package_dirs.insert(dir);
    }
    for file in &program_files {
        if file.starts_with("bundled:") {
            continue;
        }
        if key(file).contains("/node_modules/") {
            package_dirs.insert(nodepath::dirname(file).to_owned());
            if let Some(root) = package_root(file) {
                package_dirs.insert(root);
            }
            continue;
        }
        watched.flat.insert(nodepath::dirname(file).to_owned());
    }
    for dir in package_dirs {
        watched.package_dirs.insert(key(&dir));
        watched.flat.insert(dir);
    }
    // a directory inside a recursive root needs no watch of its own
    let recursive_keys: Vec<String> = watched.recursive.iter().map(|r| key(r)).collect();
    watched.flat.retain(|dir| !recursive_keys.iter().any(|r| is_under(&key(dir), r)));
    // one watch per directory, whatever its spelling
    let mut seen = FxHashSet::default();
    watched.flat.retain(|dir| seen.insert(key(dir)));
    watched
}

/// Whether an event path matters.
fn relevant(path: &str, watched: &Watched, own: &FxHashMap<String, Version>) -> bool {
    let k = key(path);
    if watched.ignored_roots.iter().any(|root| is_under(&k, root)) {
        return false;
    }
    let parent = nodepath::dirname(&k);
    let name = nodepath::basename(&k);
    let package_event = watched.module_dirs.contains(parent)
        || watched.plugin_package_dirs.contains(parent)
        || watched.plugin_package_dirs.contains(&k)
        || (watched.package_dirs.contains(parent)
            && (name == "package.json" || name.ends_with(".ts") || name.ends_with(".tsx") || name.ends_with(".mts")));
    if !package_event && k.split('/').any(|c| c == "node_modules" || c == ".git") {
        return false;
    }
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
    true
}

struct Subscriptions {
    watcher: Box<dyn Watcher>,
    active: FxHashSet<(String, bool)>,
}

impl Subscriptions {
    /// Watches what `watched` lists. An empty set (no project loaded: a broken tsconfig, a crashed build) keeps
    /// the current watches, so the fix is seen.
    fn update(&mut self, watched: &Watched) {
        if watched.recursive.is_empty() && watched.flat.is_empty() {
            return;
        }
        if debug() {
            eprintln!(
                "[rbxtsc watch] recursive {:?}
[rbxtsc watch] flat {:?}",
                watched.recursive, watched.flat
            );
        }
        let wanted: FxHashSet<(String, bool)> = watched
            .recursive
            .iter()
            .map(|d| (d.clone(), true))
            .chain(watched.flat.iter().map(|d| (d.clone(), false)))
            .filter(|(d, _)| Path::new(d).is_dir())
            .collect();
        for (dir, recursive) in self.active.iter().filter(|w| !wanted.contains(w)) {
            let _ = recursive;
            let _ = self.watcher.unwatch(Path::new(dir));
        }
        for (dir, recursive) in wanted.iter().filter(|w| !self.active.contains(w)) {
            let mode = if *recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            if let Err(e) = self.watcher.watch(Path::new(dir), mode) {
                eprintln!("rbxtsc: cannot watch {dir}: {e}");
            }
        }
        self.active = wanted;
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
    let mut subscriptions = Subscriptions { watcher, active: FxHashSet::default() };
    let mut own: FxHashMap<String, Version> = FxHashMap::default();
    // a restarted watcher goes on from the last generation of the one before
    let mut generation = if first_generation > 0 { first_generation + 1 } else { 0 };

    status("Starting compilation in watch mode...");
    rebuild(&mut session, &mut subscriptions, build, cwd, generation, &[], &mut own);
    loop {
        let Some(paths) = collect(&receiver) else { return 0 };
        let watched = watched_set(&session);
        let mut paths: Vec<String> = paths.into_iter().filter(|p| p == RESCAN || relevant(p, &watched, &own)).collect();
        // a directory as a touched path makes the session load the project again, with every file's facts
        if let Some(rescan) = paths.iter_mut().find(|p| *p == RESCAN) {
            *rescan = session.options.tsconfig.clone(); // a config input: a rebuild with every file looked at
        }
        if paths.is_empty() {
            continue;
        }
        generation += 1;
        status("File change detected. Starting incremental compilation...");
        rebuild(&mut session, &mut subscriptions, build, cwd, generation, &paths, &mut own);
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

/// Waits for an event, then for the burst to end (each event restarts the wait); the paths of the burst, sorted.
/// `None` when the watcher stopped.
fn collect(receiver: &Receiver<notify::Result<Event>>) -> Option<Vec<String>> {
    let mut paths: BTreeSet<String> = BTreeSet::new();
    let add = |event: notify::Result<Event>, paths: &mut BTreeSet<String>| match event {
        _ if debug() && {
            eprintln!("[rbxtsc watch] event {event:?}");
            false
        } => {}
        // a directory's own modification (its entries changed) says nothing its entries' events do not; a directory
        // created, moved in or renamed is kept (on Windows, a directory moved in is one event for the directory)
        Ok(event)
            if matches!(
                event.kind,
                EventKind::Modify(ModifyKind::Any | ModifyKind::Data(_) | ModifyKind::Metadata(_) | ModifyKind::Other)
            ) && event.paths.iter().all(|p| std::fs::metadata(p).is_ok_and(|m| m.is_dir())) => {}
        // events were lost (an overflowing buffer): everything may have changed
        Err(_) => {
            paths.insert(RESCAN.to_owned());
        }
        Ok(event) if event.need_rescan() => {
            paths.insert(RESCAN.to_owned());
        }
        Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
            for path in event.paths {
                paths.insert(path_text(&path));
            }
        }
        Ok(_) => {}
    };
    add(receiver.recv().ok()?, &mut paths);
    loop {
        match receiver.recv_timeout(DEBOUNCE) {
            Ok(event) => add(event, &mut paths),
            Err(RecvTimeoutError::Timeout) => return Some(paths.into_iter().collect()),
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
fn rebuild(
    session: &mut BuildSession<'_>,
    subscriptions: &mut Subscriptions,
    build: &BuildArgs,
    cwd: &str,
    generation: u64,
    paths: &[String],
    own: &mut FxHashMap<String, Version>,
) {
    if build.events_json {
        println!("{{\"event\":\"build-start\",\"gen\":{generation},\"changes\":{}}}", json_list(paths));
    }
    let result = catch_unwind(AssertUnwindSafe(|| session.build(paths)));
    if result.is_err() {
        session.reset();
    }
    subscriptions.update(&watched_set(session));
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
            return;
        }
    };
    print_report(&report, cwd, build);
    for path in &summary.written {
        if let Ok(bytes) = std::fs::read(path) {
            own.insert(key(path), Version::of(&String::from_utf8_lossy(&bytes)));
        }
    }
    let errors = error_count(&report);
    if build.events_json {
        print_end(generation, &report, &summary, errors);
    }
    status(&format!("Found {errors} error{}. Watching for file changes.", if errors == 1 { "" } else { "s" }));
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
    use super::package_root;

    #[test]
    fn package_roots() {
        assert_eq!(
            package_root("E:/p/node_modules/@rbxts/types/include/a.d.ts").as_deref(),
            Some("E:/p/node_modules/@rbxts/types")
        );
        assert_eq!(package_root("E:/p/node_modules/pkg/lib/x.d.ts").as_deref(), Some("E:/p/node_modules/pkg"));
        assert_eq!(package_root("E:/p/src/x.ts"), None);
    }
}
