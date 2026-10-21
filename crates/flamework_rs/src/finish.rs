//! `Plugin::finish` (the TS transformer's `saveArtifacts`, with `calculateGlobs`, `convertGlobs` and the empty glob
//! and path warnings): the parts of every file, in program order, become this build's `flamework.build`, the late
//! ids are resolved, and a game gets `include/flamework/{globs,config,paths}.json`.
//!
//! Every write goes through [`FinishCx`]: the compiler commits them in its one write phase, only when the build has
//! no errors. Reads of the disk here are the glob walks and the path checks of the warnings.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use rbxts_plugin::{Fatal, FileIndex, FinishCx, Utf16Pos};

use crate::build_info::{BUILD_INFO_NAME, BuildInfo};
use crate::config;
use crate::json::{self, JsObject, JsValue};
use crate::place_source::{SourcePathState, find_place_source, find_source_path};
use crate::plugin::{FwFilePart, FwShared, PathEvent};
use crate::{glob, node_path};

/// The include JSON files, in the order the TS transformer handles them.
const INCLUDE_FILES: [&str; 3] = ["globs.json", "config.json", "paths.json"];

pub(crate) fn finish(
    shared: &FwShared,
    parts: &[(FileIndex, &FwFilePart)],
    cx: &mut FinishCx<'_>,
) -> Result<(), Fatal> {
    let start = std::time::Instant::now();
    let mut info: BuildInfo = shared.build_info.clone();

    // Ids, in program order: the sequence one checker transforming the files in that order makes (DESIGN §10.4).
    let resolved = shared.ids.assign(&mut info, parts.iter().map(|(_, part)| part.new_ids.as_slice()));
    for (_, part) in parts {
        for key in &part.late_keys {
            match resolved.get(key) {
                Some(id) => cx.resolve_late(key, id),
                // Ids the build info had already were answered at once, so every late key is a new one.
                None => return Err(Fatal::new(format!("flamework: no id was assigned to {key}"))),
            }
        }
    }

    // Glob and path uses, in first-use order.
    for (_, part) in parts {
        for event in &part.events {
            match event {
                PathEvent::Glob(glob_use) => info.add_glob(&glob_use.glob, &part.file_id, Some(glob_use)),
                PathEvent::Path(path_use) => info.add_path_use(&part.file_id, path_use),
            }
        }
    }

    calculate_globs(shared, &mut info);

    if shared.is_game {
        write_include(shared, &info, cx)?;
    }

    // flamework.build, `JSON.stringify(info, undefined, "\t")`.
    cx.write_state_file(node_path::join(&[&shared.project_dir, BUILD_INFO_NAME]), info.to_json().into_bytes());

    cx.info(&format!("Flamework artifacts finished in {}ms", start.elapsed().as_millis()));

    let by_id: HashMap<&str, FileIndex> = parts.iter().map(|(i, p)| (p.file_id.as_str(), *i)).collect();
    warn_empty_globs(&info, &by_id, cx);
    warn_empty_paths(shared, &info, &by_id, cx);
    warn_unported(parts, cx);
    Ok(())
}

/// `calculateGlobs`: every glob's matches, as output paths relative to the package root. Matches are sorted (Q-F4).
fn calculate_globs(shared: &FwShared, info: &mut BuildInfo) {
    for pattern in info.glob_patterns() {
        let matches = glob::glob_sync(&pattern, Path::new(&shared.root_dir));
        let results = matches
            .iter()
            .map(|found| {
                // Relative matches are resolved against the working directory, as the path translator resolves them.
                let source = node_path::resolve(&[&shared.project_dir, found]);
                let output = shared.path_translator.get_output_path(&source);
                node_path::relative(&shared.root_dir, &output).replace('\\', "/")
            })
            .collect();
        info.set_glob_results(&pattern, results);
    }
}

/// The game's `include/flamework/*.json`: written when they have content, removed otherwise.
fn write_include(shared: &FwShared, info: &BuildInfo, cx: &mut FinishCx<'_>) -> Result<(), Fatal> {
    let mut written: BTreeMap<&str, String> = BTreeMap::new();
    let rojo = &cx.project().rojo_resolver;
    let has_rojo = shared.has_rojo;

    // The runtime sections of flamework.config.json, for the packages to read at runtime.
    if let Some(mut runtime) = config::get_runtime_config(&shared.loaded.project) {
        // `testing.entry` is a source path in the file; the runtime needs the ModuleScript's tree path.
        let entry = runtime.get("testing").and_then(|t| t.get("entry")).and_then(JsValue::as_str).map(str::to_owned);
        if let Some(entry) = entry {
            let source_path = if entry.ends_with(".ts") { entry.clone() } else { format!("{entry}.ts") };
            let output = shared.path_translator.get_output_path(&node_path::resolve(&[&shared.project_dir, &source_path]));
            let rbx_path = if has_rojo { rojo.get_rbx_path_from_file_path(&output) } else { None };
            let Some(rbx_path) = rbx_path else {
                return Err(Fatal::new(format!(
                    "flamework.config.json: testing.entry '{entry}' is not in the Rojo tree; give the source path of the ModuleScript that exports ignite(), for example \"src/server/main\""
                )));
            };
            if let Some(JsValue::Object(testing)) = runtime.as_object_mut().and_then(|r| r.get_mut("testing")) {
                testing.insert("entry", JsValue::Array(rbx_path.into_iter().map(JsValue::String).collect()));
            }
        }
        written.insert("config.json", json::stringify(&runtime, None).unwrap_or_default());
    }

    // How far below the Rojo tree's root the include folder sits.
    let include_rbx_path = if has_rojo { rojo.get_rbx_path_from_file_path(&shared.include_dir) } else { None };
    if let Some(include_rbx_path) = include_rbx_path {
        let mut paths = JsObject::new();
        paths.insert("includeDepth", JsValue::Number(include_rbx_path.len() as f64));
        written.insert("paths.json", json::stringify(&JsValue::Object(paths), None).unwrap_or_default());
    }

    let package_globs = info.children_metadata("globs");
    let globs = info.metadata("globs");
    if globs.is_some() || !package_globs.is_empty() {
        let mut game = JsObject::new();
        if let Some(paths) = globs.and_then(|g| g.get("paths")).and_then(JsValue::as_object) {
            convert_globs(shared, cx, paths, None, info, &mut game);
        }
        let mut packages = JsObject::new();
        for (prefix, package_glob) in &package_globs {
            let mut converted = JsObject::new();
            if let Some(paths) = package_glob.get("paths").and_then(JsValue::as_object) {
                convert_globs(shared, cx, paths, Some(prefix), info, &mut converted);
            }
            packages.insert(prefix.clone(), JsValue::Object(converted));
        }
        let mut out = JsObject::new();
        out.insert("game", JsValue::Object(game));
        out.insert("packages", JsValue::Object(packages));
        written.insert("globs.json", json::stringify(&JsValue::Object(out), None).unwrap_or_default());
    }

    let folder = node_path::join(&[&shared.include_dir, "flamework"]);
    for file in INCLUDE_FILES {
        let path = node_path::join(&[&folder, file]);
        match written.get(file) {
            Some(contents) => cx.write_artifact(&path, contents.clone().into_bytes()),
            // A stale file from an earlier build; the folder itself stays (TODO: the compiler removes files only).
            None if Path::new(&path).exists() => cx.remove_artifact(&path),
            None => {}
        }
    }
    Ok(())
}

/// `convertGlobs`: each glob's matches as Rojo paths. A game's globs are keyed as the code names them (obfuscated
/// under obfuscation); a package's by its own text, with its matches relative to the package.
fn convert_globs(
    shared: &FwShared,
    cx: &FinishCx<'_>,
    globs: &JsObject,
    package: Option<&str>,
    info: &BuildInfo,
    out: &mut JsObject,
) {
    let package_info = package.and_then(|p| info.build_info_from_prefix(p));
    let root = match package_info {
        Some(package_info) => node_path::dirname(&to_module_path(shared, &package_info.path)),
        None => shared.root_dir.clone(),
    };
    let rojo = &cx.project().rojo_resolver;
    for (pattern, paths) in globs.iter() {
        let mut rbx_paths = Vec::new();
        for path in paths.as_array().into_iter().flatten().filter_map(JsValue::as_str) {
            let found = if shared.has_rojo {
                rojo.get_rbx_path_from_file_path(&node_path::join(&[&root, path]))
            } else {
                None
            };
            if let Some(rbx_path) = found {
                rbx_paths.push(JsValue::Array(rbx_path.into_iter().map(JsValue::String).collect()));
            }
        }
        let key = if package_info.is_some() {
            pattern.clone()
        } else {
            shared.obfuscate_text(pattern, Some("addPaths"))
        };
        out.insert(key, JsValue::Array(rbx_paths));
    }
}

/// `toModulePath`: a package file reached through a symbolic link, back under the project's `node_modules`.
fn to_module_path(shared: &FwShared, file_path: &str) -> String {
    if node_path::is_path_descendant_of(file_path, &shared.root_dir) {
        return file_path.to_owned();
    }
    let Some(package_json) = crate::build_info::find_package_json(file_path, None) else {
        return file_path.to_owned();
    };
    let name = std::fs::read_to_string(&package_json)
        .ok()
        .and_then(|text| json::parse(text.trim_start_matches('\u{feff}')).ok())
        .and_then(|v| v.get("name").and_then(JsValue::as_str).map(str::to_owned))
        .unwrap_or_default();
    node_path::join(&[
        &shared.root_dir,
        "node_modules",
        &name,
        &node_path::relative(&node_path::dirname(&package_json), file_path),
    ])
}

fn position(line: u32, column: u32) -> Utf16Pos {
    Utf16Pos { offset: 0, line: line.saturating_sub(1), character: column.saturating_sub(1) }
}

fn number(value: &JsValue, key: &str) -> u32 {
    value.get(key).and_then(JsValue::as_f64).unwrap_or(1.0) as u32
}

fn string<'a>(value: &'a JsValue, key: &str) -> &'a str {
    value.get(key).and_then(JsValue::as_str).unwrap_or("")
}

/// `warnEmptyGlobs`: a glob that matched nothing, at every use.
fn warn_empty_globs(info: &BuildInfo, by_id: &HashMap<&str, FileIndex>, cx: &mut FinishCx<'_>) {
    let Some(globs) = info.metadata("globs") else { return };
    let (Some(paths), Some(uses)) = (globs.get("paths"), globs.get("uses").and_then(JsValue::as_object)) else {
        return;
    };
    for (origin, file_uses) in uses.iter() {
        let Some(&file) = by_id.get(origin.as_str()) else { continue };
        for glob_use in file_uses.as_array().into_iter().flatten() {
            let glob = string(glob_use, "glob");
            if paths.get(glob).and_then(JsValue::as_array).is_none_or(|m| !m.is_empty()) {
                continue;
            }
            let text = string(glob_use, "text");
            let shown = if text == glob { format!("'{glob}'") } else { format!("'{text}' ({glob})") };
            let message = format!(
                "the glob {shown} given to {} matches no files, so nothing is registered or found through it",
                string(glob_use, "macro")
            );
            cx.warn_at(file, position(number(glob_use, "line"), number(glob_use, "column")), &message, None);
        }
    }
}

/// `warnEmptyPaths`: a path macro whose Rojo path the place will not have, or will have with no module under it.
fn warn_empty_paths(shared: &FwShared, info: &BuildInfo, by_id: &HashMap<&str, FileIndex>, cx: &mut FinishCx<'_>) {
    let Some(uses) = info.metadata("paths").and_then(|p| p.get("uses")).and_then(JsValue::as_object) else { return };
    let base = shared.project_dir.clone();
    let mut states: HashMap<String, SourcePathState> = HashMap::new();
    for (origin, file_uses) in uses.iter() {
        let Some(&file) = by_id.get(origin.as_str()) else { continue };
        // In the order they are written.
        let mut ordered: Vec<&JsValue> = file_uses.as_array().into_iter().flatten().collect();
        ordered.sort_by_key(|u| (number(u, "line"), number(u, "column")));
        for path_use in ordered {
            let source_path = string(path_use, "path").to_owned();
            let state = states
                .entry(source_path.clone())
                .or_insert_with(|| look_up(shared, cx, &base, &source_path))
                .clone();
            let raises = path_use.get("raises").and_then(JsValue::as_bool) == Some(true);
            let call = format!("{}(\"{source_path}\")", string(path_use, "macro"));
            let message = match state {
                SourcePathState::Modules => continue,
                SourcePathState::Empty => format!(
                    "{call}: nothing in that folder compiles to a module, so the call finds nothing there, and in a place without the folder (git keeps no empty folder) {}",
                    if raises { "it raises at runtime after waiting five seconds for it" } else { "it waits for it at runtime" }
                ),
                SourcePathState::Missing(None) => format!(
                    "{call}: there is no such file or folder, so the place will not have it, and {}",
                    if raises { "the call raises at runtime after waiting five seconds for it" } else { "the call waits for it at runtime" }
                ),
                SourcePathState::Missing(Some(actual)) => format!(
                    "{call}: there is no such file or folder; on disk it is '{actual}', and the place names it as the disk does, so {}",
                    if raises {
                        "the call raises at runtime after waiting five seconds for a name the place does not have"
                    } else {
                        "the call waits at runtime for a name the place does not have"
                    }
                ),
            };
            cx.warn_at(file, position(number(path_use, "line"), number(path_use, "column")), &message, None);
        }
    }
}

/// Where the place gets a source path from: the deepest `$path` covering its Rojo path, else the disk.
fn look_up(shared: &FwShared, cx: &FinishCx<'_>, base: &str, source_path: &str) -> SourcePathState {
    if shared.has_rojo {
        let rojo = &cx.project().rojo_resolver;
        let output = shared.path_translator.get_output_path(&node_path::resolve(&[base, source_path]));
        if let Some(rbx_path) = rojo.get_rbx_path_from_file_path(&output) {
            let partitions: Vec<(Vec<String>, String)> =
                rojo.partitions().iter().map(|p| (p.rbx_path.clone(), p.fs_path.clone())).collect();
            if let Some(state) = find_place_source(
                &rbx_path,
                &partitions,
                &shared.path_translator.root_dir,
                &shared.path_translator.out_dir,
                base,
            ) {
                return state;
            }
        }
    }
    find_source_path(base, source_path)
}

/// One warning for the macro sites and metadata left to later milestones, at the first one.
fn warn_unported(parts: &[(FileIndex, &FwFilePart)], cx: &mut FinishCx<'_>) {
    let mut by_milestone: BTreeMap<&str, usize> = BTreeMap::new();
    let mut by_what: Vec<(&str, usize)> = Vec::new();
    let mut first = None;
    for (file, part) in parts {
        for site in &part.unported {
            *by_milestone.entry(site.milestone).or_default() += 1;
            match by_what.iter_mut().find(|(what, _)| *what == site.what) {
                Some(entry) => entry.1 += 1,
                None => by_what.push((&site.what, 1)),
            }
            first.get_or_insert((*file, site));
        }
    }
    let Some((file, site)) = first else { return };
    let total: usize = by_milestone.values().sum();
    let milestones: Vec<String> = by_milestone.iter().map(|(m, n)| format!("{m}: {n}")).collect();
    // The most frequent first; equal counts in first-seen order.
    by_what.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    let mut shown: Vec<String> = by_what.iter().take(5).map(|(what, n)| format!("{what} ({n})")).collect();
    if by_what.len() > 5 {
        shown.push(format!("{} more kinds", by_what.len() - 5));
    }
    cx.warn_at(
        file,
        site.pos,
        &format!(
            "{total} Flamework macro call(s) need features rbxts-rs does not have yet ({}); they compile as plain calls: {}",
            milestones.join(", "),
            shown.join("; ")
        ),
        Some("Their output is not what Flamework emits: do not run this build."),
    );
}
