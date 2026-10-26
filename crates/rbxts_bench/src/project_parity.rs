//! `rbxts-bench project-parity <dump.json> [--runs N] [--max-diffs N]`: compares rbxts_project's setup (Rojo
//! resolver, path translator, project data and checks) with what `tools/dump-project-services.cjs` recorded from
//! roblox-ts's JavaScript packages, value by value, for every project and program source file in the dump.
//!
//! Exit status: 0 when everything compared equal, 1 on differences, 2 on usage or load errors.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rbxts_project::rojo::RelativeRbxPathPart;
use rbxts_project::{
    FileRelation, LoadOptions, NetworkType, ProjectOptionLayer, ProjectServices, ProjectType, RbxType, RojoResolver,
    STACK_SIZE, Severity,
};
use serde_json::{Value, json};

const USAGE: &str = "usage: rbxts-bench project-parity <dump.json> [--runs N] [--max-diffs N]";

struct Args {
    dump: String,
    runs: usize,
    max_diffs: usize,
}

fn parse_args(args: Vec<String>) -> Result<Args, String> {
    let mut out = Args { dump: String::new(), runs: 5, max_diffs: 30 };
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--runs" => out.runs = value()?.parse().map_err(|e| format!("--runs: {e}"))?,
            "--max-diffs" => out.max_diffs = value()?.parse().map_err(|e| format!("--max-diffs: {e}"))?,
            "-h" | "--help" => return Err(USAGE.into()),
            _ if out.dump.is_empty() => out.dump = arg,
            _ => return Err(format!("unknown argument {arg}\n{USAGE}")),
        }
    }
    if out.dump.is_empty() {
        return Err(USAGE.into());
    }
    Ok(out)
}

/// Entry point from `main`: runs on a thread with the compiler's stack size.
pub fn main(args: Vec<String>) -> i32 {
    let args = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rbxts-bench project-parity: {e}");
            return 2;
        }
    };
    std::thread::Builder::new()
        .name("rbxts".into())
        .stack_size(STACK_SIZE)
        .spawn(move || run(&args))
        .expect("spawn the parity thread")
        .join()
        .unwrap_or(101)
}

/// Leaf-by-leaf comparison with the first differences kept.
struct Diff {
    compared: usize,
    differences: usize,
    first: Vec<String>,
    max: usize,
}

impl Diff {
    fn note(&mut self, at: &str, js: &Value, rs: &Value) {
        self.differences += 1;
        if self.first.len() < self.max {
            let short = |v: &Value| {
                let s = v.to_string();
                if s.len() > 400 {
                    format!("{}…", &s[..s.char_indices().nth(400).map_or(s.len(), |(i, _)| i)])
                } else {
                    s
                }
            };
            self.first.push(format!("{at}\n      js: {}\n      rs: {}", short(js), short(rs)));
        }
    }

    fn compare(&mut self, at: &str, js: &Value, rs: &Value) {
        match (js, rs) {
            (Value::Object(a), Value::Object(b)) => {
                for (k, va) in a {
                    let vb = b.get(k).unwrap_or(&Value::Null);
                    self.compare(&format!("{at}.{k}"), va, vb);
                }
                for (k, vb) in b {
                    if !a.contains_key(k) {
                        self.compare(&format!("{at}.{k}"), &Value::Null, vb);
                    }
                }
            }
            (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
                for (i, (va, vb)) in a.iter().zip(b).enumerate() {
                    self.compare(&format!("{at}[{i}]"), va, vb);
                }
            }
            _ => {
                self.compared += 1;
                if js != rs {
                    self.note(at, js, rs);
                }
            }
        }
    }
}

fn rbx_json(p: &Option<Vec<String>>) -> Value {
    p.as_ref().map_or(Value::Null, |p| json!(p))
}

fn network_name(n: NetworkType) -> &'static str {
    match n {
        NetworkType::Unknown => "Unknown",
        NetworkType::Client => "Client",
        NetworkType::Server => "Server",
    }
}

fn rbx_type_name(t: RbxType) -> &'static str {
    match t {
        RbxType::ModuleScript => "ModuleScript",
        RbxType::Script => "Script",
        RbxType::LocalScript => "LocalScript",
    }
}

fn relation_name(r: FileRelation) -> &'static str {
    match r {
        FileRelation::OutToOut => "OutToOut",
        FileRelation::OutToIn => "OutToIn",
        FileRelation::InToOut => "InToOut",
        FileRelation::InToIn => "InToIn",
    }
}

fn resolver_json(r: &RojoResolver) -> Value {
    let mut mounts: Vec<(&String, &Vec<String>)> = r.file_mounts().iter().collect();
    mounts.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
    json!({
        "isGame": r.is_game,
        "warnings": r.warnings(),
        "partitions": r.partitions().iter().map(|p| json!({"fsPath": p.fs_path, "rbxPath": p.rbx_path})).collect::<Vec<_>>(),
        "fileMounts": mounts.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
    })
}

fn lookup(s: &ProjectServices, path: &str) -> Value {
    let rbx_path = s.rojo_resolver.get_rbx_path_from_file_path(path);
    json!({
        "rbxPath": rbx_json(&rbx_path),
        "network": rbx_path.as_ref().map(|p| network_name(s.rojo_resolver.get_network_type(p))),
        "isolated": rbx_path.as_ref().map(|p| s.rojo_resolver.is_isolated(p)),
    })
}

/// The per-file record the dump script writes, computed with the port.
fn file_json(s: &ProjectServices, file_name: &str, is_declaration: bool, is_external: bool) -> Value {
    let pt = &s.path_translator;
    let output_path = pt.get_output_path(file_name);
    let declaration_path = pt.get_output_declaration_path(file_name);
    let import_path = pt.get_import_path(file_name, false);
    let node_module = file_name.contains("/node_modules/");
    let mapped = s.node_modules_main_for(file_name).map(str::to_owned);
    let node_import_path = pt.get_import_path(mapped.as_deref().unwrap_or(file_name), true);
    let output = lookup(s, &output_path);
    let mut f = json!({
        "fileName": file_name,
        "isDeclaration": is_declaration,
        "isExternalLibrary": is_external,
        "outputPath": output_path,
        "outputDeclarationPath": declaration_path,
        "outputTransformedPath": pt.get_output_transformed_path(file_name),
        "inputPathsOfOutput": pt.get_input_paths(&output_path),
        "inputPathsOfDeclaration": pt.get_input_paths(&declaration_path),
        "importPath": import_path,
        "importPathNodeModule": node_import_path,
        "output": output,
        "rbxType": rbx_type_name(s.rojo_resolver.get_rbx_type_from_file_path(&output_path)),
        "import": lookup(s, &import_path),
        "nodeModuleImport": if node_module { lookup(s, &node_import_path) } else { Value::Null },
        "packageRelative": if node_module { rbx_json(&s.package_relative_rbx_path(&node_import_path)) } else { Value::Null },
    });
    let rbx_path = s.rojo_resolver.get_rbx_path_from_file_path(&output_path);
    if let (Some(rbx_path), Some(runtime)) = (&rbx_path, &s.runtime_lib_rbx_path) {
        let relative: Vec<String> = RojoResolver::relative(rbx_path, runtime)
            .into_iter()
            .map(|p| match p {
                RelativeRbxPathPart::Parent => "..".to_owned(),
                RelativeRbxPathPart::Child(name) => name,
            })
            .collect();
        f["toRuntimeLib"] = json!(relative);
        f["runtimeLibRelation"] = json!(relation_name(s.rojo_resolver.get_file_relation(rbx_path, runtime)));
    }
    f
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn min_median(mut times: Vec<f64>) -> (f64, f64) {
    times.sort_by(f64::total_cmp);
    (times[0], times[times.len() / 2])
}

fn overrides_of(p: &Value) -> ProjectOptionLayer {
    let o = &p["overrides"];
    if o.is_null() {
        return ProjectOptionLayer::default();
    }
    ProjectOptionLayer {
        rojo: Some(o["rojo"].as_str().unwrap_or_default().to_owned()),
        include_path: o["includePath"].as_str().map(str::to_owned),
        project_type: o["type"].as_str().and_then(ProjectType::parse),
        luau: o["luau"].as_bool(),
        ..ProjectOptionLayer::default()
    }
}

/// The command-line overrides the dump was made with (`--rojo`, `--type`, `--includePath`).
fn cli_overrides_of(dump: &Value) -> ProjectOptionLayer {
    let o = &dump["overrides"];
    ProjectOptionLayer {
        rojo: o["rojo"].as_str().map(str::to_owned),
        include_path: o["includePath"].as_str().map(str::to_owned),
        project_type: o["type"].as_str().and_then(ProjectType::parse),
        ..ProjectOptionLayer::default()
    }
}

fn run(args: &Args) -> i32 {
    let text = match std::fs::read_to_string(&args.dump) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {}: {e}", args.dump);
            return 2;
        }
    };
    let dump: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("invalid dump: {e}");
            return 2;
        }
    };
    let report = compare_dump(&dump, args.max_diffs, args.runs);
    if report.load_failures > 0 {
        2
    } else if report.differences > 0 {
        1
    } else {
        0
    }
}

/// Totals of `compare_dump`.
#[allow(dead_code)] // `compared` and `first` are read by tests/setup_fixtures.rs
pub struct Report {
    pub compared: usize,
    pub differences: usize,
    /// The first differences, across projects.
    pub first: Vec<String>,
    /// Projects tsrs could not load.
    pub load_failures: usize,
    /// File-set mismatches, counted apart from `differences` (TypeScript 7's programs may hold other files: its
    /// `types` default is `[]`): files in only one of the two programs, and resolve probes whose `from` file the
    /// tsrs program lacks (not compared).
    pub file_set: usize,
}

/// Compares one dump with the port and prints the results. `timing_runs` 0 skips the timing.
pub fn compare_dump(dump: &Value, max_diffs: usize, timing_runs: usize) -> Report {
    struct Args {
        runs: usize,
        max_diffs: usize,
    }
    let args = Args { runs: timing_runs, max_diffs };
    let cwd = dump["cwd"].as_str().unwrap_or(".").to_owned();
    let mut total_diff = Diff { compared: 0, differences: 0, first: Vec::new(), max: args.max_diffs };
    let mut load_failures = 0;
    let mut file_set = 0;

    if !dump["error"].is_null() {
        // The JavaScript setup failed: the port must fail with the same messages.
        let tsconfig = dump["tsconfig"].as_str().unwrap_or_default();
        println!("project {tsconfig} (setup error expected)");
        match rbxts_project::load(&LoadOptions {
            tsconfig: tsconfig.to_owned(),
            cwd: cwd.clone(),
            checkers: 1,
            lazy_dts: true,
        }) {
            Ok(project) => {
                let got = match project.setup(tsconfig, &cli_overrides_of(dump)) {
                    Ok(_) => Value::Null,
                    Err(e) => json!(e.messages),
                };
                total_diff.compare("error", &dump["error"], &got);
            }
            Err(diags) => {
                println!("  tsrs could not load the tsconfig ({} diagnostics)", diags.len());
                load_failures += 1;
            }
        }
        for d in &total_diff.first {
            println!("    {d}");
        }
    }

    for p in dump["projects"].as_array().into_iter().flatten() {
        let tsconfig = p["tsconfig"].as_str().unwrap_or_default();
        let is_root = p["isRoot"].as_bool().unwrap_or(false);
        println!("project {tsconfig}{}", if is_root { " (root)" } else { " (reference)" });
        let load_start = Instant::now();
        let project = match rbxts_project::load(&LoadOptions {
            tsconfig: tsconfig.to_owned(),
            cwd: cwd.clone(),
            checkers: 1,
            lazy_dts: true,
        }) {
            Ok(p) => p,
            Err(diags) => {
                println!("  tsrs could not load the tsconfig ({} diagnostics)", diags.len());
                load_failures += 1;
                continue;
            }
        };
        let load_time = load_start.elapsed();
        let mut overrides = cli_overrides_of(dump);
        overrides.assign(&overrides_of(p));
        let setup_start = Instant::now();
        let setup = match project.setup(tsconfig, &overrides) {
            Ok(s) => s,
            Err(e) => {
                println!("  setup failed: {e}");
                total_diff.compare("error", &Value::Null, &json!(e.messages));
                continue;
            }
        };
        let setup_time = setup_start.elapsed();
        let s = &setup.services;
        let mut diff = Diff { compared: 0, differences: 0, first: Vec::new(), max: args.max_diffs };

        // project-level values
        let data = &s.data;
        diff.compare(
            "data",
            &p["data"],
            &json!({
                "projectPath": data.project_path,
                "isPackage": data.is_package,
                "nodeModulesPath": data.node_modules_path,
                "includePath": data.options.include_path,
                "rojoConfigPath": data.rojo_config_path,
                "type": data.options.project_type.map(ProjectType::as_str),
                "luau": data.options.luau,
            }),
        );
        let pt = &s.path_translator;
        diff.compare(
            "pathTranslator",
            &p["pathTranslator"],
            &json!({
                "rootDir": pt.root_dir,
                "outDir": pt.out_dir,
                "buildInfoOutputPath": pt.build_info_output_path,
                "declaration": pt.declaration,
                "useLuauExtension": pt.use_luau_extension,
            }),
        );
        // Warnings: for the root, the graph's (rbxts options, Rojo config search) and the resolver's; a reference's
        // own search is replaced by the root's Rojo config in both.
        let js_warnings: Vec<Value> = if is_root {
            dump["graphWarnings"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(p["setup"]["warnings"].as_array().into_iter().flatten())
                .cloned()
                .collect()
        } else {
            p["setup"]["warnings"].as_array().cloned().unwrap_or_default()
        };
        let rs_warnings: Vec<&String> =
            if is_root { s.warnings.iter().collect() } else { s.rojo_resolver.warnings().iter().collect() };
        diff.compare("setup.warnings", &Value::Array(js_warnings), &json!(rs_warnings));
        let diagnostics: Vec<Value> = setup
            .diagnostics
            .iter()
            .map(|d| {
                json!({"severity": if d.severity == Severity::Error { "error" } else { "warning" }, "message": d.message})
            })
            .collect();
        diff.compare("setup.diagnostics", &p["setup"]["diagnostics"], &Value::Array(diagnostics));
        diff.compare("setup.fatal", &p["setup"]["fatal"], &json!(setup.fatal));
        diff.compare("setup.projectType", &p["setup"]["projectType"], &json!(s.project_type.as_str()));
        diff.compare("setup.runtimeLibRbxPath", &p["setup"]["runtimeLibRbxPath"], &rbx_json(&s.runtime_lib_rbx_path));
        diff.compare("setup.rojo", &p["setup"]["rojo"], &resolver_json(&s.rojo_resolver));
        diff.compare(
            "setup.pkgRojo",
            &p["setup"]["pkgRojo"],
            &Value::Array(s.pkg_rojo_resolvers.iter().map(resolver_json).collect()),
        );
        let mut mapping: Vec<(&String, &String)> = s.node_modules_path_mapping.iter().collect();
        mapping.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
        diff.compare(
            "setup.nodeModulesPathMapping",
            &p["setup"]["nodeModulesPathMapping"],
            &json!(mapping.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>()),
        );
        let project_level = diff.compared;

        // per file, matched by name
        let inputs = project.input_files();
        let rs_files: HashMap<&str, _> = inputs.iter().map(|f| (f.file_name, f)).collect();
        let js_files = p["files"].as_array().cloned().unwrap_or_default();
        let js_names: std::collections::HashSet<&str> =
            js_files.iter().filter_map(|f| f["fileName"].as_str()).collect();
        let mut only_js = Vec::new();
        let mut matched = 0;
        for jf in &js_files {
            let name = jf["fileName"].as_str().unwrap_or_default();
            match rs_files.get(name) {
                Some(rf) => {
                    matched += 1;
                    let rs = file_json(s, name, rf.is_declaration, rf.is_external_library);
                    diff.compare(&format!("file {name}"), jf, &rs);
                }
                None => only_js.push(name),
            }
        }
        let only_rs: Vec<&str> = inputs.iter().map(|f| f.file_name).filter(|n| !js_names.contains(n)).collect();

        // resolve_module probes (TypeScript 5's resolveModuleName against Project::resolve_module)
        let mut probe_diff = Diff { compared: 0, differences: 0, first: Vec::new(), max: args.max_diffs };
        let probes = p["moduleProbes"].as_array().cloned().unwrap_or_default();
        let mut probes_without_from = 0;
        for probe in &probes {
            let from_name = probe["from"].as_str().unwrap_or_default();
            let Some(from) = project.program.get_source_file(from_name) else {
                probes_without_from += 1;
                continue;
            };
            let specifier = probe["specifier"].as_str().unwrap_or_default();
            let rs = project.resolve_module(specifier, from).map(|r| {
                json!({"fileName": r.file_name, "isExternalLibraryImport": r.is_external_library_import, "packageName": r.package_name})
            });
            probe_diff.compare(&format!("resolve {specifier}"), &probe["resolved"], &rs.unwrap_or(Value::Null));
        }
        println!(
            "  resolve_module probes: {}, values compared {}, differences {}, skipped (no from file) {probes_without_from}",
            probes.len(),
            probe_diff.compared,
            probe_diff.differences
        );
        for d in &probe_diff.first {
            println!("    {d}");
        }

        println!(
            "  files: {} in the JS program, {} in the tsrs program, {matched} compared; only JS {}, only tsrs {}",
            js_files.len(),
            inputs.len(),
            only_js.len(),
            only_rs.len()
        );
        for n in only_js.iter().take(5) {
            println!("    only JS:   {n}");
        }
        for n in only_rs.iter().take(5) {
            println!("    only tsrs: {n}");
        }
        println!(
            "  values compared: {} ({project_level} project-level), differences: {}",
            diff.compared, diff.differences
        );
        for d in &diff.first {
            println!("    {d}");
        }
        println!(
            "  partitions {}, file mounts {}, package resolvers {}, mapped packages {}, project type {}",
            s.rojo_resolver.partitions().len(),
            s.rojo_resolver.file_mounts().len(),
            s.pkg_rojo_resolvers.len(),
            s.node_modules_path_mapping.len(),
            s.project_type.as_str()
        );
        println!("  tsrs load {:.1} ms, setup {:.1} ms", ms(load_time), ms(setup_time));

        if is_root
            && args.runs > 0
            && let Some(rojo) = &s.data.rojo_config_path
        {
            let mut resolver_times = Vec::new();
            let mut setup_times = Vec::new();
            for _ in 0..args.runs.max(1) {
                let start = Instant::now();
                let _ = RojoResolver::from_path(rojo);
                resolver_times.push(ms(start.elapsed()));
                let start = Instant::now();
                let _ = project.setup(tsconfig, &overrides);
                setup_times.push(ms(start.elapsed()));
            }
            let (rmin, rmed) = min_median(resolver_times);
            let (smin, smed) = min_median(setup_times);
            println!("  timing (ms, min / median of {}):", args.runs.max(1));
            println!("    rust RojoResolver::from_path       {rmin:>9.2} / {rmed:>9.2}");
            for root in &s.type_roots {
                let times = (0..args.runs.max(1))
                    .map(|_| {
                        let start = Instant::now();
                        let _ = RojoResolver::synthetic(root);
                        ms(start.elapsed())
                    })
                    .collect();
                let (min, med) = min_median(times);
                let name = rbxts_project::nodepath::basename(root);
                println!("    rust synthetic({name:<20}) {min:>9.2} / {med:>9.2}");
            }
            let times = (0..args.runs.max(1))
                .map(|_| {
                    let start = Instant::now();
                    let _ = rbxts_project::setup::create_node_modules_path_mapping(&s.type_roots, false);
                    ms(start.elapsed())
                })
                .collect();
            let (min, med) = min_median(times);
            println!("    rust node_modules path mapping     {min:>9.2} / {med:>9.2}");
            println!("    rust whole setup                   {smin:>9.2} / {smed:>9.2}");
            let t = &p["timing"];
            for (key, label) in [
                ("shipped", "js fromPath (shipped)"),
                ("shippedWithNative", "js fromPath (realpathSync.native)"),
                ("installedPatched", "js fromPath (installed, patched)"),
            ] {
                if let (Some(min), Some(med)) = (t[key]["minMs"].as_f64(), t[key]["medianMs"].as_f64()) {
                    println!("    {label:<34} {min:>9.2} / {med:>9.2}");
                }
            }
        }

        // Programs can differ (TypeScript 7 defaults `types` to []); that is not the port's to match, so it is counted
        // apart
        file_set += only_js.len() + only_rs.len() + probes_without_from;
        total_diff.compared += diff.compared + probe_diff.compared;
        total_diff.differences += diff.differences + probe_diff.differences;
        for d in diff.first.into_iter().chain(probe_diff.first) {
            if total_diff.first.len() < args.max_diffs {
                total_diff.first.push(d);
            }
        }
    }
    println!(
        "total: {} values compared, {} differences, {file_set} file-set mismatches",
        total_diff.compared, total_diff.differences
    );
    Report {
        compared: total_diff.compared,
        differences: total_diff.differences,
        first: total_diff.first,
        load_failures,
        file_set,
    }
}
