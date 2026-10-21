//! `rbxts-bench build`: build a project with `rbxts_transform::compile_project` and report wall time per phase,
//! peak memory and, with `--parity <dir>`, how the outputs compare with a reference build.
//!
//! Parity classes, per project file:
//! - identical: the output equals the reference file byte for byte;
//! - differing: the file transformed cleanly but its output differs (the first differing line is shown);
//! - unsupported: the file uses node kinds the port does not handle yet (no output);
//! - failed: other errors (roblox-ts diagnostics, internal errors);
//! - missing: a reference file no project file produced, or an output the reference does not have.
//!
//! The unsupported-kind histogram counts, per kind, the files it appears in, the files where it is the only
//! unsupported kind ("sole blocker"), and its occurrences.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rbxts_project::{ProjectOptionLayer, nodepath};
use rbxts_transform::{CompileOptions, CompileReport, PluginOptions, compile_project_with};
use tsrs_compiler::diagnosticwriter::flatten_diagnostic_message;
use tsrs_diagnostics::Category;

pub const USAGE: &str = "usage: rbxts-bench build [-p tsconfig.json] [--out-dir DIR] [--optimizedLoops] [--allowCommentDirectives]\n                         [--write-partial] [--no-write] [--parity REF_DIR] [--report FILE] [--show N] [--json]\n                         [--plugin NAME]... [--timings] [--ignore-unknown-plugins] [--claims-all]\n  --plugin NAME: register a test plugin (rbxts_test_plugins), in claim order; --timings: plugin hook times;\n  --ignore-unknown-plugins: a tsconfig plugins entry no registered plugin claims is not an error;\n  --claims-all: ask every matching plugin and report double claims (as RBXTS_PLUGIN_CLAIMS=all)";

struct Args {
    tsconfig: String,
    out_dir: Option<String>,
    options: ProjectOptionLayer,
    write_partial: bool,
    no_write: bool,
    parity: Option<String>,
    report: Option<String>,
    show: usize,
    json: bool,
    plugins: Vec<String>,
    timings: bool,
    ignore_unknown_plugins: bool,
    claims_all: bool,
}

fn parse(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = args.peekable();
    let mut out = Args {
        tsconfig: "tsconfig.json".into(),
        out_dir: None,
        options: ProjectOptionLayer::default(),
        write_partial: false,
        no_write: false,
        parity: None,
        report: None,
        show: 10,
        json: false,
        plugins: Vec::new(),
        timings: false,
        ignore_unknown_plugins: false,
        claims_all: false,
    };
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "-p" | "--project" => out.tsconfig = value()?,
            "--out-dir" => out.out_dir = Some(value()?),
            "--optimizedLoops" => out.options.optimized_loops = Some(true),
            "--allowCommentDirectives" => out.options.allow_comment_directives = Some(true),
            "--noCompilerHeader" => out.options.no_compiler_header = Some(true),
            "--write-partial" => out.write_partial = true,
            "--no-write" => out.no_write = true,
            "--parity" => out.parity = Some(value()?),
            "--report" => out.report = Some(value()?),
            "--show" => out.show = value()?.parse().map_err(|e| format!("--show: {e}"))?,
            "--json" => out.json = true,
            "--plugin" => out.plugins.push(value()?),
            "--timings" => out.timings = true,
            "--ignore-unknown-plugins" => out.ignore_unknown_plugins = true,
            "--claims-all" => out.claims_all = true,
            "-h" | "--help" => return Err(USAGE.into()),
            _ => return Err(format!("unknown argument {arg}\n{USAGE}")),
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Class {
    Identical,
    Differing,
    Unsupported,
    Failed,
    Missing,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Class::Identical => "identical",
            Class::Differing => "differing",
            Class::Unsupported => "unsupported",
            Class::Failed => "failed",
            Class::Missing => "missing",
        }
    }
}

struct FileParity {
    relative: String,
    class: Class,
    detail: String,
    /// The T2 work areas of the file's unsupported kinds.
    areas: BTreeSet<&'static str>,
}

/// The work area (DESIGN §9b work split) an unsupported kind belongs to.
fn area(kind: &str) -> &'static str {
    if kind.starts_with("property call macro ")
        || kind.starts_with("call macro ")
        || kind.starts_with("identifier macro ")
    {
        return "macros (call, property call, identifier)";
    }
    if kind.contains("BindingPattern") || kind.contains("binding pattern") || kind.contains("destructuring assignment")
    {
        return "binding patterns, destructuring";
    }
    match kind {
        "NewExpression" => "new, constructor macros",
        "ClassDeclaration" | "ClassExpression" | "this in a static class member" => "classes, decorators",
        "ForOfStatement" | "ForStatement" | "WhileStatement" | "DoStatement" | "BreakStatement"
        | "ContinueStatement" => "loops, break/continue",
        "ObjectLiteralExpression" | "ArrayLiteralExpression" | "SpreadElement" => "object/array literals, spread",
        "JsxElement" | "JsxExpression" | "JsxFragment" | "JsxSelfClosingElement" => "JSX",
        "EnumDeclaration" | "ModuleDeclaration" => "enums, namespaces",
        "SwitchStatement" | "TryStatement" | "ThrowStatement" => "switch, try/throw",
        "ConditionalExpression"
        | "DeleteExpression"
        | "AwaitExpression"
        | "YieldExpression"
        | "TaggedTemplateExpression"
        | "ImportCall"
        | "logical assignment (&&=, ||=, ??=)" => "other expressions",
        _ => "other",
    }
}

/// Per area: the files using it and the files it alone blocks; then the order of areas that unblocks the most files
/// at each step (greedy), with the files unblocked so far.
fn area_report(results: &[FileParity]) -> String {
    let blocked: Vec<&BTreeSet<&str>> = results.iter().filter(|r| !r.areas.is_empty()).map(|r| &r.areas).collect();
    let mut all: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for areas in &blocked {
        for a in areas.iter() {
            let entry = all.entry(a).or_default();
            entry.0 += 1;
            if areas.len() == 1 {
                entry.1 += 1;
            }
        }
    }
    let mut text = String::new();
    let mut ranked: Vec<(&str, (usize, usize))> = all.iter().map(|(a, s)| (*a, *s)).collect();
    ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(b.1.1.cmp(&a.1.1)).then(a.0.cmp(b.0)));
    let _ = writeln!(text, "areas (files using it / files it alone blocks):");
    for (a, (files, sole)) in &ranked {
        let _ = writeln!(text, "  {files:>4} {sole:>4}  {a}");
    }
    let _ = writeln!(text, "greedy order (area: files unblocked by this step, total unblocked):");
    let mut done: BTreeSet<&str> = BTreeSet::new();
    let mut total = 0;
    while done.len() < all.len() {
        let best = all
            .keys()
            .filter(|a| !done.contains(*a))
            .map(|a| {
                let gain = blocked
                    .iter()
                    .filter(|areas| areas.contains(a) && areas.iter().all(|x| x == a || done.contains(x)))
                    .count();
                (*a, gain, all[a].0)
            })
            .max_by(|x, y| x.1.cmp(&y.1).then(x.2.cmp(&y.2)).then(y.0.cmp(x.0)))
            .expect("an area is left");
        done.insert(best.0);
        total += best.1;
        let _ = writeln!(text, "  {:>4} {total:>4}  {}", best.1, best.0);
    }
    text
}

#[derive(Default)]
struct KindStats {
    files: usize,
    sole: usize,
    occurrences: usize,
}

fn first_difference(ours: &str, reference: &str) -> String {
    let (a, b): (Vec<&str>, Vec<&str>) = (ours.split('\n').collect(), reference.split('\n').collect());
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i), b.get(i));
        if x != y {
            return format!(
                "line {}: ours {:?} / ref {:?}",
                i + 1,
                x.map_or("<end>", |s| s.trim_end_matches('\r')),
                y.map_or("<end>", |s| s.trim_end_matches('\r'))
            );
        }
    }
    "same lines (line endings or trailing bytes differ)".to_owned()
}

fn collect_files(dir: &Path, base: &Path, out: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, base, out);
        } else if path.extension().is_some_and(|e| e == "luau" || e == "lua") {
            let relative = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            out.insert(relative);
        }
    }
}

fn parity(
    report: &CompileReport,
    out_dir: &str,
    reference_dir: &str,
) -> (Vec<FileParity>, BTreeMap<String, KindStats>) {
    let mut results = Vec::new();
    let mut produced = BTreeSet::new();
    let mut kinds: BTreeMap<String, KindStats> = BTreeMap::new();
    for outcome in &report.files {
        let relative = nodepath::relative(out_dir, &outcome.out_path).replace('\\', "/");
        produced.insert(relative.clone());
        let reference_path = PathBuf::from(reference_dir).join(&relative);
        let reference = std::fs::read_to_string(&reference_path).ok();
        let unsupported: BTreeSet<&String> = outcome.unsupported.iter().collect();
        for kind in &unsupported {
            let stats = kinds.entry((*kind).clone()).or_default();
            stats.files += 1;
            if unsupported.len() == 1 {
                stats.sole += 1;
            }
        }
        for kind in &outcome.unsupported {
            kinds.entry(kind.clone()).or_default().occurrences += 1;
        }
        let (class, detail) = if !unsupported.is_empty() {
            (Class::Unsupported, unsupported.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
        } else if !outcome.is_clean() {
            let first = outcome
                .diagnostics
                .iter()
                .find(|d| d.category() == Category::Error)
                .map(|d| flatten_diagnostic_message(*d, " "))
                .unwrap_or_default();
            (Class::Failed, first)
        } else {
            match (&outcome.luau, &reference) {
                (Some(ours), Some(reference)) if ours == reference => (Class::Identical, String::new()),
                (Some(ours), Some(reference)) => (Class::Differing, first_difference(ours, reference)),
                (Some(_), None) => (Class::Missing, "not in the reference".to_owned()),
                (None, _) => (Class::Failed, "no output".to_owned()),
            }
        };
        let areas = unsupported.iter().map(|k| area(k)).collect();
        results.push(FileParity { relative, class, detail, areas });
    }
    let mut reference_files = BTreeSet::new();
    collect_files(Path::new(reference_dir), Path::new(reference_dir), &mut reference_files);
    for relative in reference_files.difference(&produced) {
        results.push(FileParity {
            relative: relative.clone(),
            class: Class::Missing,
            detail: "no project file produced it".to_owned(),
            areas: BTreeSet::new(),
        });
    }
    results.sort_by(|a, b| a.class.cmp(&b.class).then_with(|| a.relative.cmp(&b.relative)));
    (results, kinds)
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

pub fn run(args: impl Iterator<Item = String>) -> i32 {
    let args = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rbxts-bench build: {e}");
            return 2;
        }
    };
    let mut plugins = Vec::new();
    for name in &args.plugins {
        // `flamework`: the Flamework plugin (flamework_rs), the one real plugin; the rest are the seam's test plugins.
        match rbxts_test_plugins::by_name(name).or_else(|| (name == "flamework").then(|| rbxts_plugin::erased::erase(flamework_rs::Flamework::new()))) {
            Some(plugin) => plugins.push(plugin),
            None => {
                eprintln!(
                    "rbxts-bench build: unknown plugin {name:?}; known: {}",
                    rbxts_test_plugins::NAMES.join(", ")
                );
                return 2;
            }
        }
    }
    let plugin_options = PluginOptions {
        plugins: &plugins,
        ignore_unknown_tsconfig_plugins: args.ignore_unknown_plugins,
        claims_all: args.claims_all,
    };
    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let start = std::time::Instant::now();
    let options = CompileOptions {
        tsconfig: args.tsconfig.clone(),
        cwd,
        project_options: args.options.clone(),
        out_dir_override: args.out_dir.clone(),
        write_partial: args.write_partial || args.parity.is_some(),
        no_write: args.no_write,
    };
    let report = compile_project_with(&options, &plugin_options);
    let total = start.elapsed();
    let (peak_ws, _, peak_commit) = crate::memory().unwrap_or((0, 0, 0));

    let t = &report.timings;
    let phases = [
        ("config", t.config),
        ("program", t.program),
        ("setup", t.setup),
        ("check", t.check),
        ("plugin_setup", t.plugin_setup),
        ("transform", t.transform),
        ("plugin_finish", t.plugin_finish),
        ("render", t.render),
        ("load_check", t.load_check),
        ("write", t.write),
    ];
    let clean = report.files.iter().filter(|f| f.is_clean()).count();
    let written = report.files.iter().filter(|f| f.written).count();
    let diagnostics = report.diagnostics.len() + report.files.iter().map(|f| f.diagnostics.len()).sum::<usize>();

    let mut out = String::new();
    if args.json {
        let phase_json: Vec<String> = phases.iter().map(|(n, d)| format!("\"{n}\":{:.1}", ms(*d))).collect();
        let _ = write!(
            out,
            "{{\"files\":{},\"project_files\":{},\"clean\":{clean},\"written\":{written},\"diagnostics\":{diagnostics},\"total_ms\":{:.1},\"peak_ws_mib\":{:.1},\"peak_commit_mib\":{:.1},\"phases\":{{{}}}}}",
            report.program_files,
            report.project_files,
            ms(total),
            crate::mib(peak_ws),
            crate::mib(peak_commit),
            phase_json.join(",")
        );
        println!("{out}");
        out.clear();
    } else {
        let _ = writeln!(
            out,
            "files {} (project {}), transformed cleanly {clean}, written {written}, diagnostics {diagnostics}",
            report.program_files, report.project_files
        );
        for (name, d) in phases {
            let _ = writeln!(out, "  {name:<13} {:>9.1} ms", ms(d));
        }
        let _ = writeln!(out, "  {:<13} {:>9.1} ms", "total", ms(total));
        if t.load_checked > 0 {
            let _ = writeln!(out, "Luau load check: {} files near a limit", t.load_checked);
        }
        let _ = writeln!(
            out,
            "peak working set {:.1} MiB, peak commit {:.1} MiB",
            crate::mib(peak_ws),
            crate::mib(peak_commit)
        );
        for message in &report.fatal {
            let _ = writeln!(out, "fatal: {message}");
        }
        for d in report.diagnostics.iter().take(args.show) {
            let file = d.file().map(|f| f.file_name().to_owned()).unwrap_or_default();
            let _ = writeln!(out, "  {file}: {}", flatten_diagnostic_message(*d, " "));
        }
        if !report.plugins.is_empty() {
            let _ = writeln!(out, "plugins: {}", report.plugins.join(", "));
            for warning in &report.warnings {
                let _ = writeln!(out, "  warning: {}", warning.replace('\n', " / "));
            }
            for d in report.files.iter().flat_map(|f| f.diagnostics.iter()).take(args.show) {
                let file = d.file().map(|f| f.file_name().to_owned()).unwrap_or_default();
                let category = if d.category() == Category::Error { "error" } else { "warning" };
                let _ = writeln!(out, "  {category} {file}: {}", flatten_diagnostic_message(*d, " "));
            }
            for path in &report.artifacts_written {
                let _ = writeln!(out, "  wrote {path}");
            }
            for path in &report.artifacts_deleted {
                let _ = writeln!(out, "  deleted {path}");
            }
        }
        if args.timings {
            for plugin in &report.plugin_timings {
                let _ = writeln!(out, "plugin {}", plugin.name);
                for (hook, time) in &plugin.hooks {
                    if time.calls > 0 {
                        let label = hook.label();
                        let _ = writeln!(out, "  {label:<28} {:>8} calls {:>10.3} ms", time.calls, ms(time.time));
                    }
                }
            }
        }
        print!("{out}");
    }

    let mut status = i32::from(report.has_errors());
    if let Some(reference_dir) = &args.parity {
        let out_dir = report.out_dir.clone();
        let (results, kinds) = parity(&report, &out_dir, reference_dir);
        let mut counts: BTreeMap<Class, usize> = BTreeMap::new();
        for r in &results {
            *counts.entry(r.class).or_default() += 1;
        }
        let mut text = String::new();
        let summary: Vec<String> =
            [Class::Identical, Class::Differing, Class::Unsupported, Class::Failed, Class::Missing]
                .iter()
                .map(|c| format!("{} {}", c.label(), counts.get(c).copied().unwrap_or(0)))
                .collect();
        let _ = writeln!(text, "parity vs {reference_dir}: {}", summary.join(", "));
        let mut ranked: Vec<(&String, &KindStats)> = kinds.iter().collect();
        ranked.sort_by(|a, b| b.1.files.cmp(&a.1.files).then(b.1.sole.cmp(&a.1.sole)).then(a.0.cmp(b.0)));
        let _ = writeln!(text, "unsupported kinds (files using it / files where it is the only one / occurrences):");
        for (kind, stats) in &ranked {
            let _ = writeln!(text, "  {:>4} {:>4} {:>6}  {kind}", stats.files, stats.sole, stats.occurrences);
        }
        text.push_str(&area_report(&results));
        let mut full = text.clone();
        for r in &results {
            let line = format!("{:<11} {}  {}", r.class.label(), r.relative, r.detail);
            let _ = writeln!(full, "{line}");
        }
        let mut shown = 0;
        for r in &results {
            if matches!(r.class, Class::Differing | Class::Failed | Class::Missing) && shown < args.show {
                let _ = writeln!(text, "{:<11} {}  {}", r.class.label(), r.relative, r.detail);
                shown += 1;
            }
        }
        print!("{text}");
        if let Some(report_path) = &args.report
            && let Err(e) = std::fs::write(report_path, full)
        {
            eprintln!("rbxts-bench build: could not write {report_path}: {e}");
        }
        if counts.get(&Class::Identical).copied().unwrap_or(0) != results.len() {
            status = status.max(1);
        }
    }
    status
}
