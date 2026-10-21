//! `rbxts-bench watch-parity`: the W1 harness (DESIGN §14) over a project and a JSON edit script
//! (`rbxts_transform::build::parity`). After every step the in-process watcher's tree must equal a clean build of
//! the same sources, seeded with the watcher's plugin states; a failing step must write nothing and report the clean
//! build's errors. Prints one line per step with the watcher's and the clean build's times.

use std::path::PathBuf;

use rbxts_project::ProjectOptionLayer;
use rbxts_transform::build::parity::{NodeModules, ParityOptions, parse_script, run};

pub const USAGE: &str = "usage: rbxts-bench watch-parity --source DIR [-p tsconfig.json] --script FILE --scratch DIR\n                         [--copy NAME,NAME…] [--optimizedLoops] [--allowCommentDirectives] [--plugin NAME]...\n                         [--keep-clean] [--report FILE] [--ignore-unknown-plugins] [--node-modules DIR]\n                         [--incremental] [--copy-node-modules | --link-packages] [--no-clean]\n  --source: the project to copy (never written); --copy: the entries copied into the watched copy (default\n  src,include,tsconfig.json,default.project.json,package.json; node_modules is linked); --scratch: where the\n  copies go (emptied first); --keep-clean: keep each step's clean copy; --no-clean: no clean\n  builds (timings and memory only); --incremental: each step is a new\n  incremental build (\"incremental\": true is added to the copy's tsconfig) instead of a watch rebuild.";

pub fn main(args: Vec<String>) -> i32 {
    let mut source = None;
    let mut tsconfig = "tsconfig.json".to_owned();
    let mut script = None;
    let mut scratch = None;
    let mut copy: Vec<String> =
        ["src", "include", "tsconfig.json", "default.project.json", "package.json"].map(str::to_owned).to_vec();
    let mut options = ProjectOptionLayer::default();
    let mut plugin_names = Vec::new();
    let mut keep = false;
    let mut ignore_unknown_plugins = false;
    let mut node_modules = None;
    let mut incremental = false;
    let mut node_modules_mode = NodeModules::Link;
    let mut no_clean = false;
    let mut report = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        let parsed: Result<(), String> = (|| {
            match arg.as_str() {
                "--source" => source = Some(PathBuf::from(value()?)),
                "-p" | "--project" => tsconfig = value()?,
                "--script" => script = Some(value()?),
                "--scratch" => scratch = Some(PathBuf::from(value()?)),
                "--copy" => copy = value()?.split(',').map(str::to_owned).collect(),
                "--optimizedLoops" => options.optimized_loops = Some(true),
                "--allowCommentDirectives" => options.allow_comment_directives = Some(true),
                "--plugin" => plugin_names.push(value()?),
                "--keep-clean" => keep = true,
                "--ignore-unknown-plugins" => ignore_unknown_plugins = true,
                "--node-modules" => node_modules = Some(PathBuf::from(value()?)),
                "--incremental" => incremental = true,
                "--copy-node-modules" => node_modules_mode = NodeModules::Copy,
                "--link-packages" => node_modules_mode = NodeModules::LinkPackages,
                "--no-clean" => no_clean = true,
                "--report" => report = Some(value()?),
                _ => return Err(format!("unknown argument {arg}\n{USAGE}")),
            }
            Ok(())
        })();
        if let Err(e) = parsed {
            eprintln!("{e}");
            return 2;
        }
    }
    let (Some(source), Some(script), Some(scratch)) = (source, script, scratch) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let text = match std::fs::read_to_string(&script) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("{script}: {e}");
            return 2;
        }
    };
    let script_value = match parse_script(&text) {
        Ok(script) => script,
        Err(e) => {
            eprintln!("{script}: {e}");
            return 2;
        }
    };
    let mut plugins = Vec::new();
    for name in &plugin_names {
        match rbxts_test_plugins::by_name(name) {
            Some(plugin) => plugins.push(plugin),
            None => {
                eprintln!("unknown test plugin {name} (known: {})", rbxts_test_plugins::NAMES.join(", "));
                return 2;
            }
        }
    }
    let options = ParityOptions {
        source,
        tsconfig,
        copy,
        scratch,
        project_options: options,
        plugins: &plugins,
        ignore_unknown_plugins,
        node_modules,
        node_modules_mode,
        incremental,
        keep_clean_copies: keep,
        check_clean: !no_clean,
        memory: Some(|| crate::memory().map(|(_, working_set, _)| working_set)),
    };
    let results = match run(&options, &script_value) {
        Ok(results) => results,
        Err(e) => {
            eprintln!("watch-parity: {e}");
            return 1;
        }
    };
    let mut out = String::new();
    let mut failed = 0;
    for (index, result) in results.iter().enumerate() {
        let verdict = if result.problems.is_empty() { "PASS" } else { "FAIL" };
        out.push_str(&format!(
            "{verdict} {index:>2} {:<44} {:<5} {:<13} transformed {:>4}{} watch {:>8.1} ms  clean {:>8.1} ms  parsed {}\n",
            result.name,
            match result.kind {
                Some(rbxts_plugin::BuildKind::Clean) => "clean".to_owned(),
                Some(rbxts_plugin::BuildKind::Incremental) => "incremental".to_owned(),
                Some(rbxts_plugin::BuildKind::WatchRebuild { generation }) => format!("rebuild {generation}"),
                None => "-".to_owned(),
            },
            if result.ok { "ok" } else { "error" },
            result.transformed,
            if result.all { " (all)" } else { "" },
            result.watch_ms,
            result.clean_ms,
            result.parsed,
        ));
        let t = &result.timings;
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
        out.push_str(&format!(
            "       program {:.0} setup {:.0} invalidate {:.0} check {:.0} transform {:.0} render {:.0} write {:.0}  \
             working set {} MiB\n",
            ms(t.config + t.program),
            ms(t.setup + t.plugin_setup),
            ms(t.invalidate),
            ms(t.check),
            ms(t.transform + t.plugin_finish),
            ms(t.render),
            ms(t.write),
            result.memory.map_or("-".to_owned(), |m| (m >> 20).to_string())
        ));
        for problem in &result.problems {
            out.push_str(&format!("       {problem}\n"));
        }
        failed += usize::from(!result.problems.is_empty());
    }
    out.push_str(&format!("watch-parity: {} steps, {failed} failed\n", results.len()));
    print!("{out}");
    if let Some(report) = report
        && let Err(e) = std::fs::write(&report, &out)
    {
        eprintln!("{report}: {e}");
    }
    i32::from(failed > 0)
}
