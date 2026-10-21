//! An `rbxtsc`-compatible command line (roblox-ts's `CLI/commands/build.ts`) over `rbxts_transform`, with
//! compiled-in plugins (DESIGN §11.6):
//!
//! ```no_run
//! fn main() -> std::process::ExitCode {
//!     rbxts_cli::Cli::new()
//!         // .plugin(flamework_rs::Flamework::new())
//!         .run()
//! }
//! ```
//!
//! [`Cli::run`] parses roblox-ts's `build` arguments (`-p`, `--verbose`, `--rojo`, `--includePath`, `--type`,
//! `--optimizedLoops`, `--allowCommentDirectives`, `--noInclude`, `--logTruthyChanges`, `--writeOnlyChanged`,
//! `--writeTransformedFiles`, `--luau`), builds on a thread with a 512 MiB stack, prints the diagnostics as
//! TypeScript's pretty reporter does, and returns roblox-ts's exit codes: 0 when the build succeeded, 1 otherwise.
//! `-w` / `--watch` runs watch mode ([`watch`], DESIGN §14), with `--usePolling` and `--events json` (machine-readable
//! build events on stdout).
//!
//! `RBXTS_PLUGIN_CLAIMS=all` asks every matching plugin for each claimable node and reports double claims.

pub mod watch;

use std::ffi::OsString;
use std::process::ExitCode;

use rbxts_plugin::Plugin;
use rbxts_plugin::erased::{DynPlugin, erase};
use rbxts_project::{ProjectOptionLayer, ProjectType, STACK_SIZE, nodepath};
use rbxts_transform::{CompileOptions, CompileReport, PluginOptions, compile_project_with};
use tsrs_compiler::diagnosticwriter::{FormattingOptions, format_diagnostics_with_color_and_context};
use tsrs_core::tspath::ComparePathsOptions;
use tsrs_diagnostics::Category;

const USAGE: &str = "\
rbxtsc build [options]

Build a project

Options:
  -p, --project                 project path                      [string] [default: \".\"]
      --verbose                 enable verbose logs                            [boolean]
      --noInclude               do not copy include files                      [boolean]
      --logTruthyChanges        logs changes to truthiness evaluation from Lua truthiness rules
                                                                               [boolean]
      --type                    override project type   [choices: \"game\", \"model\", \"package\"]
  -i, --includePath             folder to copy runtime files to                 [string]
      --rojo                    manually select Rojo project file               [string]
  -w, --watch                   enable watch mode                              [boolean]
      --usePolling              use polling for watch mode                     [boolean]
      --events                  machine-readable watch events on stdout   [choices: \"json\"]
      --luau                    emit files with .luau extension                [boolean]
      --timings                 print plugin hook timings                      [boolean]
  -h, --help                    show help
  -v, --version                 show version number";

/// The command line, with the plugins compiled into this binary.
#[derive(Default)]
pub struct Cli {
    plugins: Vec<Box<dyn DynPlugin>>,
}

/// Parsed `build` arguments.
#[derive(Debug, Default, PartialEq)]
pub struct BuildArgs {
    pub project: String,
    pub verbose: bool,
    pub timings: bool,
    /// `-w` / `--watch`.
    pub watch: bool,
    /// `--usePolling`: poll instead of file system notifications.
    pub use_polling: bool,
    /// `--events json`.
    pub events_json: bool,
    pub options: ProjectOptionLayer,
}

/// What the arguments ask for.
#[derive(Debug, PartialEq)]
pub enum Command {
    Build(BuildArgs),
    Help,
    Version,
}

/// Splits `--name=value` into the name and the value.
fn split_flag(arg: &str) -> (&str, Option<&str>) {
    match arg.split_once('=') {
        Some((name, value)) if name.starts_with('-') => (name, Some(value)),
        _ => (arg, None),
    }
}

fn parse_bool(name: &str, value: Option<&str>) -> Result<bool, String> {
    match value {
        None | Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(other) => Err(format!("Invalid value for {name}: {other:?} (expected true or false)")),
    }
}

/// Parses roblox-ts's `build` arguments (the leading `build` is optional, as `$0` is its alias). Paths are resolved
/// against `cwd`, as `path.resolve` does in roblox-ts.
pub fn parse_args(args: impl IntoIterator<Item = String>, cwd: &str) -> Result<Command, String> {
    let mut args = args.into_iter().peekable();
    if args.peek().is_some_and(|a| a == "build") {
        args.next();
    }
    let mut out = BuildArgs { project: ".".to_owned(), ..BuildArgs::default() };
    while let Some(arg) = args.next() {
        let (name, inline) = split_flag(&arg);
        let mut value = |name: &str| -> Result<String, String> {
            match inline {
                Some(v) => Ok(v.to_owned()),
                None => args
                    .next()
                    .ok_or_else(|| format!("Not enough arguments following: {}", name.trim_start_matches('-'))),
            }
        };
        let resolve = |path: String| nodepath::resolve(&[cwd, &path]);
        match name {
            "-h" | "--help" => return Ok(Command::Help),
            "-v" | "--version" => return Ok(Command::Version),
            "-p" | "--project" => out.project = value(name)?,
            "--verbose" => out.verbose = parse_bool(name, inline)?,
            "--timings" => out.timings = parse_bool(name, inline)?,
            "-w" | "--watch" => out.watch = parse_bool(name, inline)?,
            "--usePolling" => out.use_polling = parse_bool(name, inline)?,
            "--events" => {
                let v = value(name)?;
                if v != "json" {
                    return Err(format!("Invalid values:\n  Argument: events, Given: {v:?}, Choices: \"json\""));
                }
                out.events_json = true;
            }
            "--noInclude" => out.options.no_include = Some(parse_bool(name, inline)?),
            "--logTruthyChanges" => out.options.log_truthy_changes = Some(parse_bool(name, inline)?),
            "--writeOnlyChanged" => out.options.write_only_changed = Some(parse_bool(name, inline)?),
            "--writeTransformedFiles" => out.options.write_transformed_files = Some(parse_bool(name, inline)?),
            "--optimizedLoops" => out.options.optimized_loops = Some(parse_bool(name, inline)?),
            "--allowCommentDirectives" => out.options.allow_comment_directives = Some(parse_bool(name, inline)?),
            "--luau" => out.options.luau = Some(parse_bool(name, inline)?),
            "--type" => {
                let v = value(name)?;
                let t = ProjectType::parse(&v).ok_or_else(|| {
                    format!(
                        "Invalid values:\n  Argument: type, Given: {v:?}, Choices: \"game\", \"model\", \"package\""
                    )
                })?;
                out.options.project_type = Some(t);
            }
            "-i" | "--includePath" => out.options.include_path = Some(resolve(value(name)?)),
            "--rojo" => out.options.rojo = Some(resolve(value(name)?)),
            _ => return Err(format!("Unknown argument: {}", arg.trim_start_matches('-'))),
        }
    }
    Ok(Command::Build(out))
}

impl Cli {
    /// A command line with no plugins.
    pub fn new() -> Cli {
        Cli::default()
    }

    /// Registers a plugin. Plugins are asked in registration order; a plugin with a `tsconfig_transform` is on only
    /// for projects whose tsconfig `plugins` list that transform.
    pub fn plugin<P: Plugin>(mut self, plugin: P) -> Cli {
        self.plugins.push(erase(plugin));
        self
    }

    /// Registers an already erased plugin.
    pub fn erased_plugin(mut self, plugin: Box<dyn DynPlugin>) -> Cli {
        self.plugins.push(plugin);
        self
    }

    /// Runs with the process's arguments.
    pub fn run(self) -> ExitCode {
        let args: Vec<String> =
            std::env::args_os().skip(1).map(|a: OsString| a.to_string_lossy().into_owned()).collect();
        ExitCode::from(self.run_with(args))
    }

    /// Runs with these arguments (without the program name) and returns the exit status: 0 or 1.
    pub fn run_with(self, args: Vec<String>) -> u8 {
        let thread =
            std::thread::Builder::new().name("rbxtsc".into()).stack_size(STACK_SIZE).spawn(move || self.main(args));
        match thread.map(|t| t.join()) {
            Ok(Ok(status)) => status,
            Ok(Err(_)) => {
                eprintln!("rbxtsc: the compiler crashed (a panic outside the per-file transform)");
                1
            }
            Err(e) => {
                eprintln!("rbxtsc: could not start the build thread: {e}");
                1
            }
        }
    }

    fn main(self, args: Vec<String>) -> u8 {
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let build = match parse_args(args, &cwd) {
            Ok(Command::Build(build)) => build,
            Ok(Command::Help) => {
                println!("{USAGE}");
                return 0;
            }
            Ok(Command::Version) => {
                println!("{}", rbxts_transform::shared::constants::COMPILER_VERSION);
                return 0;
            }
            Err(message) => {
                eprintln!("{USAGE}\n\n{message}");
                return 1;
            }
        };
        let tsconfig = match rbxts_project::setup::find_tsconfig_path(&build.project, &cwd) {
            Ok(path) => path,
            Err(_) => {
                eprintln!("Unable to find tsconfig.json!");
                return 1;
            }
        };
        let options = CompileOptions {
            tsconfig,
            cwd: cwd.clone(),
            project_options: build.options.clone(),
            out_dir_override: None,
            write_partial: false,
            no_write: false,
        };
        if build.watch {
            return watch::run(&self.plugins, &build, options, &cwd);
        }
        let plugin_options = PluginOptions { plugins: &self.plugins, ..PluginOptions::default() };
        let report = compile_project_with(&options, &plugin_options);
        print_report(&report, &cwd, &build);
        u8::from(report.has_errors())
    }
}

/// Prints the build's outcome: warnings, plugin messages (`--verbose`), diagnostics with TypeScript's pretty
/// reporter, fatal errors, and the plugin timings (`--timings`).
pub fn print_report(report: &CompileReport, cwd: &str, build: &BuildArgs) {
    for warning in &report.warnings {
        println!("\u{1b}[33m{warning}\u{1b}[0m");
    }
    if build.verbose {
        for message in &report.plugin_messages {
            println!("{message}");
        }
    }
    let mut diagnostics = report.diagnostics.clone();
    for file in &report.files {
        diagnostics.extend(file.diagnostics.iter().copied());
    }
    if !diagnostics.is_empty() {
        let options = FormattingOptions {
            compare_paths_options: ComparePathsOptions {
                use_case_sensitive_file_names: false,
                current_directory: tsrs_core::tspath::normalize_path(cwd),
            },
            new_line: "\n".to_owned(),
        };
        let mut out = Vec::new();
        format_diagnostics_with_color_and_context(&mut out, &diagnostics, &options);
        print!("{}", String::from_utf8_lossy(&out));
        if !out.ends_with(b"\n") {
            println!();
        }
    }
    for message in &report.fatal {
        eprintln!("\u{1b}[31mProject Error:\u{1b}[0m {message}");
    }
    if build.verbose {
        let errors = diagnostics.iter().filter(|d| d.category() == Category::Error).count();
        let written = report.files.iter().filter(|f| f.written).count();
        println!("{} files, {written} written, {errors} errors", report.files.len());
    }
    if build.timings {
        print!("{}", format_plugin_timings(report));
    }
}

/// The plugin hook timings as a table: plugin, hook, calls, milliseconds (hooks that never ran are left out).
pub fn format_plugin_timings(report: &CompileReport) -> String {
    let mut out = String::new();
    for plugin in &report.plugin_timings {
        out.push_str(&format!("plugin {}\n", plugin.name));
        for (hook, time) in &plugin.hooks {
            if time.calls > 0 {
                out.push_str(&format!(
                    "  {:<28} {:>8} calls {:>10.2} ms\n",
                    hook.label(),
                    time.calls,
                    time.time.as_secs_f64() * 1e3
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(|s| s.to_string()), "E:/work")
    }

    #[test]
    fn build_arguments() {
        let Command::Build(build) = parse(&[
            "build",
            "-p",
            "game",
            "--verbose",
            "--optimizedLoops",
            "--allowCommentDirectives=true",
            "--type",
            "package",
            "-i",
            "include",
            "--rojo=default.project.json",
            "--luau=false",
        ])
        .unwrap() else {
            panic!("a build")
        };
        assert_eq!(build.project, "game");
        assert!(build.verbose);
        assert_eq!(build.options.optimized_loops, Some(true));
        assert_eq!(build.options.allow_comment_directives, Some(true));
        assert_eq!(build.options.project_type, Some(ProjectType::Package));
        assert_eq!(build.options.include_path.as_deref(), Some(nodepath::resolve(&["E:/work", "include"]).as_str()));
        assert_eq!(
            build.options.rojo.as_deref(),
            Some(nodepath::resolve(&["E:/work", "default.project.json"]).as_str())
        );
        assert_eq!(build.options.luau, Some(false));
    }

    #[test]
    fn defaults_help_and_errors() {
        assert_eq!(parse(&[]).unwrap(), Command::Build(BuildArgs { project: ".".into(), ..BuildArgs::default() }));
        assert_eq!(parse(&["--help"]).unwrap(), Command::Help);
        assert_eq!(parse(&["-v"]).unwrap(), Command::Version);
        let Command::Build(watch) = parse(&["-w", "--usePolling", "--events", "json"]).unwrap() else { panic!() };
        assert!(watch.watch && watch.use_polling && watch.events_json);
        assert!(parse(&["--events", "xml"]).unwrap_err().contains("Choices"));
        assert!(parse(&["--type", "place"]).unwrap_err().contains("Choices"));
        assert!(parse(&["--bogus"]).unwrap_err().contains("Unknown argument"));
        assert!(parse(&["-p"]).unwrap_err().contains("Not enough arguments"));
    }
}
