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
//! `--writeTransformedFiles`, `--luau`) as yargs does, builds on a thread with a 512 MiB stack, and returns
//! roblox-ts's exit codes: 0 when the build succeeded, 1 otherwise. Output follows roblox-ts's CLI: on stdout, every
//! diagnostic as TypeScript's pretty reporter prints it (fatal errors and "Unable to find tsconfig.json!" as roblox-ts
//! text diagnostics), and argument errors as yargs's message alone. `-w` / `--watch` runs watch mode ([`watch`],
//! DESIGN §14), with `--usePolling` and `--events json` (machine-readable build events on stdout).
//!
//! `RBXTS_PLUGIN_CLAIMS=all` asks every matching plugin for each claimable node and reports double claims.

pub mod watch;

use std::ffi::OsString;
use std::process::ExitCode;

use rbxts_plugin::Plugin;
use rbxts_plugin::erased::{DynPlugin, erase};
use rbxts_project::{ProjectOptionLayer, ProjectType, STACK_SIZE, nodepath};
use rbxts_transform::shared::diagnostics::text_diagnostic;
use rbxts_transform::{CompileOptions, CompileReport, PluginOptions, compile_project_with};
use tsrs_ast::Diagnostic;
use tsrs_compiler::diagnosticwriter::{FormattingOptions, format_diagnostic_with_color_and_context};
use tsrs_core::P;
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

/// The boolean options and their negated form (`--no-<name>`, yargs's boolean negation).
fn boolean_flag(name: &str) -> Option<(&str, bool)> {
    const BOOLEANS: &[&str] = &[
        "--verbose",
        "--timings",
        "--noInclude",
        "--logTruthyChanges",
        "--writeOnlyChanged",
        "--writeTransformedFiles",
        "--optimizedLoops",
        "--allowCommentDirectives",
        "--luau",
        "--watch",
        "--usePolling",
    ];
    if let Some(&flag) = BOOLEANS.iter().find(|&&flag| flag == name) {
        return Some((flag, true));
    }
    let negated = format!("--{}", name.strip_prefix("--no-")?);
    BOOLEANS.iter().find(|&&flag| flag == negated).map(|&flag| (flag, false))
}

/// Parses roblox-ts's `build` arguments (the leading `build` is optional, as `$0` is its alias) as yargs does: a
/// boolean takes `=true` / `=false` or a following `true` / `false`, and `--no-<name>` negates it; a string option
/// takes `=value` or the next argument unless that is a flag, else it is empty. Paths are resolved against `cwd`, as
/// `path.resolve` does in roblox-ts; an empty `--includePath` or `--rojo` means the default, as there. Errors carry
/// yargs's message.
pub fn parse_args(args: impl IntoIterator<Item = String>, cwd: &str) -> Result<Command, String> {
    let mut args = args.into_iter().peekable();
    if args.peek().is_some_and(|a| a == "build") {
        args.next();
    }
    let mut out = BuildArgs { project: ".".to_owned(), ..BuildArgs::default() };
    while let Some(arg) = args.next() {
        let (name, inline) = split_flag(&arg);
        if !name.starts_with('-') {
            return Err(format!("Unknown argument: {name}"));
        }
        if let Some((flag, positive)) = boolean_flag(if name == "-w" { "--watch" } else { name }) {
            let value = match inline {
                Some(v) => Some(v.to_owned()),
                None if args.peek().is_some_and(|a| a == "true" || a == "false") => args.next(),
                None => None,
            };
            let value = match value.as_deref() {
                None | Some("true") => positive,
                Some("false") => !positive,
                Some(other) => return Err(format!("Invalid value for {flag}: {other:?} (expected true or false)")),
            };
            match flag {
                "--verbose" => out.verbose = value,
                "--timings" => out.timings = value,
                "--noInclude" => out.options.no_include = Some(value),
                "--logTruthyChanges" => out.options.log_truthy_changes = Some(value),
                "--writeOnlyChanged" => out.options.write_only_changed = Some(value),
                "--writeTransformedFiles" => out.options.write_transformed_files = Some(value),
                "--optimizedLoops" => out.options.optimized_loops = Some(value),
                "--allowCommentDirectives" => out.options.allow_comment_directives = Some(value),
                "--watch" => out.watch = value,
                "--usePolling" => out.use_polling = value,
                _ => out.options.luau = Some(value),
            }
            continue;
        }
        // a string option's value: inline, or the next argument unless it is a flag (None: no value at all)
        let mut value = || -> Option<String> {
            match inline {
                Some(v) => Some(v.to_owned()),
                None if args.peek().is_some_and(|a| !a.starts_with('-')) => args.next(),
                None => None,
            }
        };
        let path = |value: Option<String>| match value.unwrap_or_default() {
            empty if empty.is_empty() => empty,
            path => nodepath::resolve(&[cwd, &path]),
        };
        match name {
            "-h" | "--help" => return Ok(Command::Help),
            "-v" | "--version" => return Ok(Command::Version),
            "-p" | "--project" => out.project = value().unwrap_or_default(),
            "--events" => {
                let v = value();
                if v.as_deref() != Some("json") {
                    let given = v.as_ref().map_or_else(|| "true".to_owned(), |v| format!("{v:?}"));
                    return Err(format!("Invalid values:\n  Argument: events, Given: {given}, Choices: \"json\""));
                }
                out.events_json = true;
            }
            "--type" => {
                let v = value();
                let t = v.as_deref().and_then(ProjectType::parse).ok_or_else(|| {
                    let given = v.as_ref().map_or_else(|| "true".to_owned(), |v| format!("{v:?}"));
                    format!(
                        "Invalid values:\n  Argument: type, Given: {given}, Choices: \"game\", \"model\", \"package\""
                    )
                })?;
                out.options.project_type = Some(t);
            }
            "-i" | "--includePath" => out.options.include_path = Some(path(value())),
            "--rojo" => out.options.rojo = Some(path(value())),
            _ => return Err(format!("Unknown argument: {}", name.trim_start_matches('-'))),
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
                // yargs's `.fail()` in roblox-ts's CLI: the message alone, on stdout
                println!("{message}");
                return 1;
            }
        };
        let tsconfig = match rbxts_project::setup::find_tsconfig_path(&build.project, &cwd) {
            Ok(path) => path,
            Err(_) => {
                // a `CLIError`, logged with `LogService.writeLine`: the formatted diagnostic, then "\n"
                let error = text_diagnostic("Unable to find tsconfig.json!", Category::Error);
                println!("{}", format_diagnostic(error, &cwd));
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

/// `ts.sys.newLine`.
const NEW_LINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// One diagnostic as TypeScript's `formatDiagnosticsWithColorAndContext` formats it (ending with a line end), with
/// the OS's line ends. rbxts-rs: roblox-ts's own diagnostics, ad hoc messages here, show roblox-ts's code
/// (`TS roblox-ts`, from `createTextDiagnostic` and `createDiagnosticWithLocation`) instead of tsrs's `TS-1`.
pub fn format_diagnostic(diagnostic: P<Diagnostic>, cwd: &str) -> String {
    let options = FormattingOptions {
        compare_paths_options: ComparePathsOptions {
            use_case_sensitive_file_names: false,
            current_directory: tsrs_core::tspath::normalize_path(cwd),
        },
        new_line: NEW_LINE.to_owned(),
    };
    let mut out = Vec::new();
    format_diagnostic_with_color_and_context(&mut out, diagnostic, &options);
    let mut text = String::from_utf8_lossy(&out).into_owned();
    if diagnostic.code() == -1 {
        text = text.replacen("\u{1b}[90m TS-1: ", "\u{1b}[90m TS roblox-ts: ", 1);
    }
    if !text.ends_with(NEW_LINE) {
        text.push_str(NEW_LINE);
    }
    text
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
    // roblox-ts reports its fatal errors (`ProjectError`, `emitResultFailure`) as text diagnostics too
    let fatal = report.fatal.iter().map(|message| text_diagnostic(message, Category::Error));
    for diagnostic in diagnostics.iter().copied().chain(fatal) {
        // `ts.createDiagnosticReporter(ts.sys, true)`: the formatted diagnostic, then a line end
        print!("{}{NEW_LINE}", format_diagnostic(diagnostic, cwd));
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
        assert!(parse(&["-w", "false"]).is_ok_and(|c| matches!(c, Command::Build(b) if !b.watch)));
        assert!(parse(&["--type", "place"]).unwrap_err().contains("Given: \"place\", Choices"));
        assert!(parse(&["--type"]).unwrap_err().contains("Given: true, Choices"));
        assert_eq!(parse(&["--bogus=1"]).unwrap_err(), "Unknown argument: bogus");
        assert_eq!(parse(&["-x"]).unwrap_err(), "Unknown argument: x");
        assert_eq!(parse(&["build", "build"]).unwrap_err(), "Unknown argument: build");
    }

    /// yargs's value rules, as roblox-ts's CLI parses them.
    #[test]
    fn yargs_values() {
        let build = |args: &[&str]| match parse(args).unwrap() {
            Command::Build(build) => build,
            other => panic!("a build, not {other:?}"),
        };
        // a string option without a value is empty: `-p` alone builds the working directory
        assert_eq!(build(&["-p"]).project, "");
        let b = build(&["-p", "--noInclude"]);
        assert_eq!((b.project.as_str(), b.options.no_include), ("", Some(true)));
        // an empty include path or Rojo file means the default
        assert_eq!(build(&["-i"]).options.include_path.as_deref(), Some(""));
        assert_eq!(build(&["--includePath="]).options.include_path.as_deref(), Some(""));
        assert_eq!(build(&["--rojo", "--verbose"]).options.rojo.as_deref(), Some(""));
        // booleans: a following true/false, `=value`, and `--no-<name>`
        assert_eq!(build(&["--noInclude", "false"]).options.no_include, Some(false));
        assert_eq!(build(&["--noInclude=false"]).options.no_include, Some(false));
        assert_eq!(build(&["--no-noInclude"]).options.no_include, Some(false));
        assert_eq!(build(&["--no-luau"]).options.luau, Some(false));
        assert!(build(&["--verbose", "true"]).verbose);
        assert!(parse(&["--verbose=maybe"]).unwrap_err().contains("expected true or false"));
    }
}
