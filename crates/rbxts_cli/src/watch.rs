//! Watch mode (`rbxtsc -w`, DESIGN §14 "Watcher"): one [`BuildSession`] rebuilt on file system events.
//!
//! - **Events.** A `notify` watcher (ReadDirectoryChangesW on Windows, or polling with `--usePolling`) on the root
//!   directories (recursive), the tsconfig's directory and the directories of the configs it extends, of the Rojo
//!   project files and of the plugins' input files (not recursive), the directories plugins asked to watch
//!   (`watch_dir`, recursive), and the directories of program files outside those (not `node_modules`). Events
//!   under the output roots, the include folder, `node_modules` and `.git`, rbxts-rs's own temporary files, and
//!   writes of this watcher's own artifacts are dropped.
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
    /// Path keys of the `node_modules` directories watched for plugin module resolutions (their events count).
    package_dirs: FxHashSet<String>,
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
            watched.package_dirs.insert(key(&dir));
            watched.flat.insert(dir);
        }
    }
    for file in program_files {
        let k = key(&file);
        if k.contains("/node_modules/") || file.starts_with("bundled:") {
            continue;
        }
        watched.flat.insert(nodepath::dirname(&file).to_owned());
    }
    // a directory inside a recursive root needs no watch of its own
    let recursive_keys: Vec<String> = watched.recursive.iter().map(|r| key(r)).collect();
    watched.flat.retain(|dir| !recursive_keys.iter().any(|r| is_under(&key(dir), r)));
    watched
}

/// Whether an event path matters.
fn relevant(path: &str, watched: &Watched, own: &FxHashMap<String, Version>) -> bool {
    let k = key(path);
    if watched.ignored_roots.iter().any(|root| is_under(&k, root)) {
        return false;
    }
    let in_package_dir = watched.package_dirs.contains(nodepath::dirname(&k)) || watched.package_dirs.contains(&k);
    if !in_package_dir && k.split('/').any(|c| c == "node_modules" || c == ".git") {
        return false;
    }
    // a directory's own events (created, its entries changed) say nothing its files' events do not, except a
    // directory that is gone (its files went with it) or a watched package directory
    if !in_package_dir && std::fs::metadata(path).is_ok_and(|m| m.is_dir()) {
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
    fn update(&mut self, watched: &Watched) {
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
    rebuild(&mut session, build, cwd, generation, &[], &mut own);
    subscriptions.update(&watched_set(&session));
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
        rebuild(&mut session, build, cwd, generation, &paths, &mut own);
        subscriptions.update(&watched_set(&session));
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
fn rebuild(
    session: &mut BuildSession<'_>,
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
    let (report, summary) = match result {
        Ok(result) => result,
        Err(panic) => {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "panic".to_owned());
            eprintln!("\u{1b}[31mrbxtsc: the build crashed:\u{1b}[0m {message}\nThe next build starts clean.");
            session.reset();
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
