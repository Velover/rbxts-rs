//! Measurement harness.
//! - `rbxts-bench load` (milestone 0): load a project through tsrs in-process, type-check its files, and report wall
//!   time per phase and peak memory.
//! - `rbxts-bench luau-parity`: render Luau trees dumped from roblox-ts (`tools/dump-luau-ast.cjs`) with
//!   `rbxts_luau` and compare the text byte for byte.
//! - `rbxts-bench project-parity`: compare rbxts_project's project setup (Rojo resolver, path translator, checks) with
//!   roblox-ts's on the same project (`tools/dump-project-services.cjs`), value by value.
//!
//! - `rbxts-bench build`: build a project with `rbxts_transform` (check, transform, render, write), with phase
//!   timings, peak memory and an optional parity comparison against a reference output directory.
//!
//! rbxts-bench build [-p tsconfig.json] [--out-dir DIR] [--parity REF_DIR] [--report FILE] …
//! rbxts-bench load [-p tsconfig.json] [--checkers N] [--lazy-dts] [--check project|all|none] [--json]
//! rbxts-bench luau-parity <dump.jsonl>... [--show N] [--repeat N] [--threads N]
//! rbxts-bench project-parity <dump.json> [--runs N] [--max-diffs N]

mod build;
mod divergences;
mod luau_parity;
mod project_parity;
mod watch_parity;

#[global_allocator]
static GLOBAL: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;

use std::time::{Duration, Instant};

use rbxts_project::{LoadOptions, STACK_SIZE};
use tsrs_ast::Diagnostic;
use tsrs_compiler::Context;
use tsrs_compiler::diagnosticwriter::flatten_diagnostic_message;
use tsrs_core::P;
use tsrs_diagnostics::Category;

#[derive(PartialEq)]
enum CheckScope {
    Project,
    All,
    None,
}

struct Args {
    tsconfig: String,
    checkers: usize,
    lazy_dts: bool,
    check: CheckScope,
    json: bool,
}

const USAGE: &str = "usage: rbxts-bench load [-p tsconfig.json] [--checkers N] [--check project|all|none] [--lazy-dts] [--json]\n       rbxts-bench build [-p tsconfig.json] [--out-dir DIR] [--parity REF_DIR] [--report FILE] [--json] (see `rbxts-bench build --help`)\n       rbxts-bench luau-parity <dump.jsonl>... [--show N] [--repeat N] [--threads N]\n       rbxts-bench project-parity <dump.json> [--runs N] [--max-diffs N]
       rbxts-bench watch-parity --source DIR --script FILE --scratch DIR (see `rbxts-bench watch-parity --help`)";

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("load") => {}
        Some("-h" | "--help") | None => return Err(USAGE.into()),
        Some(other) => return Err(format!("unknown command `{other}`\n{USAGE}")),
    }
    let mut out = Args {
        tsconfig: "tsconfig.json".into(),
        checkers: 1,
        lazy_dts: false,
        check: CheckScope::Project,
        json: false,
    };
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "-p" | "--project" => out.tsconfig = value()?,
            "--checkers" => out.checkers = value()?.parse().map_err(|e| format!("--checkers: {e}"))?,
            "--lazy-dts" => out.lazy_dts = true,
            "--check" => {
                out.check = match value()?.as_str() {
                    "project" => CheckScope::Project,
                    "all" => CheckScope::All,
                    "none" => CheckScope::None,
                    v => return Err(format!("--check {v}: expected project, all or none")),
                }
            }
            "--json" => out.json = true,
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if out.checkers == 0 {
        return Err("--checkers must be at least 1".into());
    }
    if out.lazy_dts && out.check == CheckScope::All {
        return Err("--lazy-dts is only valid when declaration files are not checked".into());
    }
    Ok(out)
}

/// Peak and current working set, and peak commit (private bytes), in bytes.
#[cfg(windows)]
pub(crate) fn memory() -> Option<(usize, usize, usize)> {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn K32GetProcessMemoryInfo(process: *mut c_void, counters: *mut ProcessMemoryCounters, cb: u32) -> i32;
    }
    let mut counters = ProcessMemoryCounters::default();
    let cb = size_of::<ProcessMemoryCounters>() as u32;
    counters.cb = cb;
    // SAFETY: the pseudo-handle needs no closing; `counters` is a valid, correctly sized out-parameter.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, cb) };
    (ok != 0).then_some((counters.peak_working_set_size, counters.working_set_size, counters.peak_pagefile_usage))
}

#[cfg(not(windows))]
pub(crate) fn memory() -> Option<(usize, usize, usize)> {
    None
}

struct Phase {
    name: &'static str,
    time: Duration,
    /// Peak working set when the phase ended; None when it was not sampled right after the phase.
    peak_ws: Option<usize>,
}

fn record(phases: &mut Vec<Phase>, name: &'static str, time: Duration) {
    let peak_ws = memory().map(|m| m.0);
    phases.push(Phase { name, time, peak_ws });
}

fn message(d: P<Diagnostic>) -> String {
    flatten_diagnostic_message(d, "
")
}

/// Where a diagnostic is, as TypeScript's plain formatter puts it: `file(line,column)` (1-based, the column in UTF-16
/// units), the file alone without a position, nothing without a file.
pub(crate) fn location(d: P<Diagnostic>) -> String {
    use tsrs_ast::SourceFileLike;
    let Some(file) = d.file() else { return String::new() };
    let file = file.get();
    let name = file.file_name();
    let Some(pos) = d.pos() else { return name.to_owned() };
    let starts = file.ecma_line_map();
    let line = starts.partition_point(|&start| start <= pos).saturating_sub(1);
    let text = SourceFileLike::text(file);
    let start = starts.get(line).map_or(0, |&s| s as usize).min(text.len());
    let column = text.get(start..(pos as usize).min(text.len())).map_or(0, |s| s.encode_utf16().count());
    format!("{name}({},{})", line + 1, column + 1)
}

pub(crate) fn mib(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn run(args: &Args) -> i32 {
    let total = Instant::now();
    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let project = match rbxts_project::load(&LoadOptions {
        tsconfig: args.tsconfig.clone(),
        cwd,
        checkers: args.checkers,
        lazy_dts: args.lazy_dts,
    regions: false,
    }) {
        Ok(p) => p,
        Err(diags) => {
            eprintln!("tsconfig could not be used ({} diagnostics)", diags.len());
            for d in diags {
                eprintln!("  {}: {}", location(d), message(d));
            }
            return 2;
        }
    };
    // `load` runs both phases, so the memory sample after it belongs to the program phase only.
    let mut phases = vec![Phase { name: "config", time: project.config_time, peak_ws: None }];
    record(&mut phases, "program (read+parse+bind+resolve)", project.program_time);

    let ctx = Context::background();
    let all_files = project.program.source_files();
    let project_files = project.project_files();
    let to_check = match args.check {
        CheckScope::Project => project_files.clone(),
        CheckScope::All => all_files.to_vec(),
        CheckScope::None => Vec::new(),
    };

    let start = Instant::now();
    let mut diags = project.program_diagnostics();
    diags.extend(project.check_files(&ctx, &to_check));
    record(&mut phases, "check", start.elapsed());
    let total = total.elapsed();

    let errors = diags.iter().filter(|d| d.category() == Category::Error).count();
    let (peak_ws, ws, peak_commit) = memory().unwrap_or((0, 0, 0));
    if args.json {
        let phase_json: Vec<String> = phases
            .iter()
            .map(|p| {
                let peak = p.peak_ws.map_or_else(|| "null".to_owned(), |b| format!("{:.1}", mib(b)));
                format!("{{\"phase\":\"{}\",\"ms\":{:.1},\"peak_ws_mib\":{peak}}}", p.name, p.time.as_secs_f64() * 1e3)
            })
            .collect();
        println!(
            "{{\"checkers\":{},\"checkers_effective\":{},\"lazy_dts\":{},\"files\":{},\"project_files\":{},\"checked\":{},\"diagnostics\":{},\"errors\":{},\"total_ms\":{:.1},\"peak_ws_mib\":{:.1},\"ws_mib\":{:.1},\"peak_commit_mib\":{:.1},\"phases\":[{}]}}",
            args.checkers,
            project.checker_count(),
            args.lazy_dts,
            all_files.len(),
            project_files.len(),
            to_check.len(),
            diags.len(),
            errors,
            total.as_secs_f64() * 1e3,
            mib(peak_ws),
            mib(ws),
            mib(peak_commit),
            phase_json.join(","),
        );
    } else {
        println!(
            "files {} (project {}), checked {}, checkers {} (effective {}), lazy dts {}",
            all_files.len(),
            project_files.len(),
            to_check.len(),
            args.checkers,
            project.checker_count(),
            args.lazy_dts
        );
        for p in &phases {
            let peak = p.peak_ws.map_or_else(String::new, |b| format!("   peak working set {:>7.1} MiB", mib(b)));
            println!("  {:<34} {:>9.1} ms{peak}", p.name, p.time.as_secs_f64() * 1e3);
        }
        println!("  {:<34} {:>9.1} ms", "total", total.as_secs_f64() * 1e3);
        println!("diagnostics {} (errors {})", diags.len(), errors);
        println!("peak working set {:.1} MiB, peak commit {:.1} MiB", mib(peak_ws), mib(peak_commit));
        for d in diags.iter().take(20) {
            println!("  {}: {}", location(*d), message(*d));
        }
    }
    i32::from(errors > 0)
}

fn main() {
    rbxts_project::use_tsrs_fast_preset();
    if std::env::args().nth(1).as_deref() == Some("watch-parity") {
        let status = std::thread::Builder::new()
            .name("watch-parity".into())
            .stack_size(STACK_SIZE)
            .spawn(|| watch_parity::main(std::env::args().skip(2).collect()))
            .expect("spawn the harness thread")
            .join()
            .unwrap_or(1);
        std::process::exit(status);
    }
    if std::env::args().nth(1).as_deref() == Some("project-parity") {
        std::process::exit(project_parity::main(std::env::args().skip(2).collect()));
    }
    if std::env::args().nth(1).as_deref() == Some("build") {
        let status = std::thread::Builder::new()
            .name("build".into())
            .stack_size(STACK_SIZE)
            .spawn(|| build::run(std::env::args().skip(2)))
            .expect("spawn the build thread")
            .join()
            .unwrap_or(101);
        std::process::exit(status);
    }
    if std::env::args().nth(1).as_deref() == Some("luau-parity") {
        // Deep trees recurse deeply, as checking does.
        let status = std::thread::Builder::new()
            .name("luau-parity".into())
            .stack_size(STACK_SIZE)
            .spawn(|| luau_parity::run(std::env::args().skip(2)))
            .expect("spawn the parity thread")
            .join()
            .unwrap_or(101);
        std::process::exit(status);
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rbxts-bench: {e}");
            std::process::exit(2);
        }
    };
    let status = std::thread::Builder::new()
        .name("rbxts".into())
        .stack_size(STACK_SIZE)
        .spawn(move || run(&args))
        .expect("spawn the compile thread")
        .join()
        .unwrap_or(101);
    std::process::exit(status);
}
