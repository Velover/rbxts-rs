//! The project's outputs beyond the Luau of its source files (roblox-ts's `Project/functions/getProjectOutputs.ts`):
//! the assets copied from the root directories, the set of files the clean keeps, and the clean that deletes every
//! other file under the output roots after a successful build ("stale outputs": the outputs of deleted or renamed
//! sources, and anything else roblox-ts would remove).
//!
//! rbxts-rs: differences from roblox-ts:
//! - assets are written only when their bytes changed (DESIGN §14 commit step 1), not copied on every build;
//! - the include folder is kept whole when it lies under an output root (DESIGN §14: "The include folder is in the
//!   clean step's keep set"), not only the runtime files roblox-ts copies (rbxts-rs does not copy them yet);
//! - plugin artifacts and state files of this build are kept;
//! - declaration files: rbxts-rs emits none yet, so project `.d.ts` files are not transformed and copied, and their
//!   `.d.ts` outputs are kept as roblox-ts keeps them.

use std::path::Path;

use rustc_hash::{FxHashMap, FxHashSet};

use rbxts_project::{ProjectServices, nodepath};

use crate::plugins::lifecycle::{remove_file, write_if_changed};

/// The outputs of one build.
#[derive(Debug, Default)]
pub struct ProjectOutputs {
    /// The roots the clean walks: the outDir (or its override) and the `declarationDir`.
    pub roots: Vec<String>,
    /// The path keys of every file the clean keeps.
    pub keep: FxHashSet<String>,
    /// Directories the clean never enters (the include path).
    pub keep_dirs: Vec<String>,
    /// Assets: (input, output), copied when their bytes differ.
    pub assets: Vec<(String, String)>,
}

/// A path as a comparison key: normalised, forward slashes, lowercased unless the file system is case-sensitive.
pub fn path_key(services: &ProjectServices, path: &str) -> String {
    let normalized = nodepath::normalize(path).replace('\\', "/");
    if services.use_case_sensitive_file_names { normalized } else { normalized.to_lowercase() }
}

/// `isPathDescendantOf` on keys: `path` is `root` or inside it.
fn is_descendant_key(path: &str, root: &str) -> bool {
    path == root || (path.len() > root.len() && path.starts_with(root) && path.as_bytes()[root.len()] == b'/')
}

/// What [`get_project_outputs`] needs to know of the build.
pub struct OutputInputs<'a> {
    pub services: &'a ProjectServices,
    /// The program's compiler options (root directories, declarations).
    pub options: &'a tsrs_core::CompilerOptions,
    /// The project files' output paths (after the override).
    pub luau_outputs: &'a [String],
    /// The outDir override of the build, if any.
    pub out_dir_override: Option<&'a str>,
    /// tsconfig and `extends` paths, never assets.
    pub config_paths: &'a [String],
    /// Further files to keep: build info, plugin artifacts and state files of this build.
    pub extra_keep: &'a [String],
}

/// Moves an output path under the override, as the Luau outputs are.
fn overridden(services: &ProjectServices, path: &str, out_dir_override: Option<&str>) -> String {
    match out_dir_override {
        Some(dir)
            if is_descendant_key(&path_key(services, path), &path_key(services, &services.path_translator.out_dir)) =>
        {
            let relative = nodepath::relative(&services.path_translator.out_dir, path);
            nodepath::join(&[dir, &relative])
        }
        _ => path.to_owned(),
    }
}

/// `getProjectOutputs`: the roots, the keep set and the assets. Fails like roblox-ts when an output root contains a
/// source root ("Output directory … contains source directory …"), which also keeps the clean away from sources, and
/// when two inputs emit to one output.
pub fn get_project_outputs(inputs: &OutputInputs<'_>) -> Result<ProjectOutputs, String> {
    let services = inputs.services;
    let translator = &services.path_translator;
    let mut outputs = ProjectOutputs::default();
    let out_root = inputs.out_dir_override.map_or_else(|| translator.out_dir.clone(), str::to_owned);
    outputs.roots.push(out_root.clone());
    if inputs.options.get_emit_declarations() && !inputs.options.declaration_dir.is_empty() {
        let declaration_dir = nodepath::normalize(&inputs.options.declaration_dir);
        if inputs.out_dir_override.is_none() {
            outputs.roots.push(declaration_dir);
        }
    }

    let root_dirs: Vec<String> = {
        let mut dirs = rbxts_project::setup::get_root_dirs(inputs.options);
        if dirs.is_empty() && inputs.options.composite.is_true() {
            dirs.push(services.data.project_path.clone());
        }
        dirs.iter().map(|d| nodepath::resolve(&[d])).collect()
    };
    for root in &outputs.roots {
        let root_key = path_key(services, root);
        for source in &root_dirs {
            if is_descendant_key(&path_key(services, source), &root_key) {
                return Err(format!("Output directory \"{root}\" contains source directory \"{source}\"."));
            }
        }
    }

    let mut owners: FxHashMap<String, String> = FxHashMap::default();
    let mut add_output = |outputs: &mut ProjectOutputs, output: &str, input: &str| -> Result<(), String> {
        let key = path_key(services, output);
        if let Some(previous) = owners.get(&key)
            && previous != input
        {
            return Err(format!("Files \"{previous}\" and \"{input}\" both emit to \"{output}\"."));
        }
        owners.insert(key.clone(), input.to_owned());
        outputs.keep.insert(key);
        Ok(())
    };
    for output in inputs.luau_outputs {
        outputs.keep.insert(path_key(services, output));
    }
    if translator.declaration {
        // rbxts-rs: no declaration emit yet; keep what a declaration build would write
        for output in inputs.luau_outputs {
            let stem = output.strip_suffix(".luau").or_else(|| output.strip_suffix(".lua")).unwrap_or(output);
            outputs.keep.insert(path_key(services, &format!("{stem}.d.ts")));
        }
    }

    let include = nodepath::resolve(&[&services.data.options.include_path]);
    let mut excluded: Vec<String> = outputs.roots.iter().map(|r| path_key(services, r)).collect();
    excluded.push(path_key(services, &translator.out_dir));
    excluded.push(path_key(services, &include));
    let config_keys: FxHashSet<String> = inputs.config_paths.iter().map(|p| path_key(services, p)).collect();
    let build_info = translator.build_info_output_path.as_ref().map(|p| path_key(services, p));
    let project_dir_key = path_key(services, &services.data.project_path);

    // (path, is a directory); entries' types come from the directory listing (links are followed)
    let mut stack: Vec<(String, bool)> =
        root_dirs.iter().map(|d| (d.clone(), std::fs::metadata(d).is_ok_and(|m| m.is_dir()))).collect();
    let mut assets = Vec::new();
    while let Some((input, is_dir)) = stack.pop() {
        let key = path_key(services, &input);
        if excluded.iter().any(|root| is_descendant_key(&key, root)) {
            continue;
        }
        if is_dir {
            let name = nodepath::basename(&input);
            if name == "node_modules" || name == ".git" {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&input) else { continue };
            let mut children = Vec::new();
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else { continue };
                let is_dir = if file_type.is_symlink() {
                    std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir())
                } else {
                    file_type.is_dir()
                };
                children.push((nodepath::join(&[&input, &entry.file_name().to_string_lossy()]), is_dir));
            }
            // pushed in reverse, so the walk visits them in directory order
            children.reverse();
            stack.extend(children);
            continue;
        }
        let base = nodepath::basename(&input);
        let is_deployment_config = path_key(services, nodepath::dirname(&input)) == project_dir_key
            && base.ends_with(".project.json")
            && base.len() > 13;
        if config_keys.contains(&key) || build_info.as_ref() == Some(&key) || is_deployment_config {
            continue;
        }
        if input.ends_with(".ts") || input.ends_with(".tsx") {
            // rbxts-rs: `.d.ts` assets need declaration transforms, not ported
            continue;
        }
        let output = overridden(services, &translator.get_output_path(&input), inputs.out_dir_override);
        add_output(&mut outputs, &output, &input)?;
        assets.push((input, output));
    }
    assets.sort();
    outputs.assets = assets;

    if let Some(build_info) = &translator.build_info_output_path {
        outputs.keep.insert(path_key(services, build_info));
    }
    for extra in inputs.extra_keep {
        outputs.keep.insert(path_key(services, extra));
    }
    let rojo = &services.rojo_resolver;
    for config in rojo.config_files() {
        outputs.keep.insert(path_key(services, config));
    }
    outputs.keep_dirs.push(include);
    Ok(outputs)
}

/// Commit step 1 for assets: copies each asset whose output is missing or differs. Returns the outputs written.
pub fn sync_assets(outputs: &ProjectOutputs) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for (input, output) in &outputs.assets {
        let bytes = std::fs::read(input).map_err(|e| format!("could not read {input}: {e}"))?;
        if write_if_changed(Path::new(output), &bytes).map_err(|e| format!("could not write {output}: {e}"))? {
            written.push(output.clone());
        }
    }
    Ok(written)
}

/// The files under the output roots before a build writes anything, taken once per build: for the missing-output
/// check and for the clean. The include folder and `.git` are not entered.
#[derive(Debug, Default)]
pub struct Listing {
    /// (path, key) of every file.
    pub files: Vec<(String, String)>,
    pub keys: FxHashSet<String>,
    /// The directories that were empty (roblox-ts's clean removes empty directories too).
    pub empty_dirs: Vec<String>,
}

/// The output roots of a build: the outDir (or its override), and the `declarationDir` of a declaration build
/// without an override.
pub fn output_roots(
    services: &ProjectServices,
    options: &tsrs_core::CompilerOptions,
    out_dir_override: Option<&str>,
) -> Vec<String> {
    let mut roots = vec![out_dir_override.map_or_else(|| services.path_translator.out_dir.clone(), str::to_owned)];
    if out_dir_override.is_none() && options.get_emit_declarations() && !options.declaration_dir.is_empty() {
        roots.push(nodepath::normalize(&options.declaration_dir));
    }
    roots
}

/// Lists the files under `roots`.
pub fn list(services: &ProjectServices, roots: &[String]) -> Listing {
    let include = path_key(services, &nodepath::resolve(&[&services.data.options.include_path]));
    let mut listing = Listing::default();
    let mut stack: Vec<String> = roots.iter().filter(|r| Path::new(r.as_str()).is_dir()).cloned().collect();
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else { continue };
        let mut count = 0;
        for entry in entries.flatten() {
            count += 1;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            let path = nodepath::join(&[&directory, &name]);
            let Ok(file_type) = entry.file_type() else { continue };
            let key = path_key(services, &path);
            if file_type.is_dir() {
                if !is_descendant_key(&key, &include) {
                    stack.push(path);
                }
            } else {
                listing.keys.insert(key.clone());
                listing.files.push((path, key));
            }
        }
        if count == 0 {
            listing.empty_dirs.push(directory);
        }
    }
    listing
}

/// Commit step 3 for stale outputs: deletes every listed file that is not kept, then the directories that leaves
/// empty (never a root). Returns the files deleted.
pub fn clean(services: &ProjectServices, outputs: &ProjectOutputs, listing: &Listing) -> Result<Vec<String>, String> {
    let roots: Vec<String> = outputs.roots.iter().map(|r| path_key(services, r)).collect();
    let mut deleted = Vec::new();
    // the directories that may be left empty: every ancestor of a deleted file below its root
    let mut candidates: FxHashMap<String, String> = FxHashMap::default();
    for (path, key) in &listing.files {
        let Some(root) = roots.iter().find(|root| is_descendant_key(key, root)) else { continue };
        if outputs.keep.contains(key) {
            continue;
        }
        if remove_file(Path::new(path)).map_err(|e| format!("could not delete {path}: {e}"))? {
            deleted.push(path.clone());
        }
        let mut directory = nodepath::dirname(path).to_owned();
        loop {
            let directory_key = path_key(services, &directory);
            if directory_key == *root || !is_descendant_key(&directory_key, root) {
                break;
            }
            let parent = nodepath::dirname(&directory).to_owned();
            let shorter = parent.len() < directory.len();
            if candidates.insert(directory_key, directory).is_some() || !shorter {
                break;
            }
            directory = parent;
        }
    }
    for directory in &listing.empty_dirs {
        candidates.insert(path_key(services, directory), directory.clone());
        let mut parent = nodepath::dirname(directory).to_owned();
        while let Some(root) = roots.iter().find(|root| is_descendant_key(&path_key(services, &parent), root)) {
            let key = path_key(services, &parent);
            if key == *root || candidates.insert(key, parent.clone()).is_some() {
                break;
            }
            parent = nodepath::dirname(&parent).to_owned();
        }
    }
    candidates.retain(|key, _| !roots.contains(key));
    // deepest first, so a parent is looked at after its children
    let mut candidates: Vec<String> = candidates.into_values().collect();
    candidates.sort_by_key(|d| std::cmp::Reverse(d.split(['/', '\\']).count()));
    for directory in candidates {
        if std::fs::read_dir(&directory).is_ok_and(|mut d| d.next().is_none()) {
            std::fs::remove_dir(&directory).map_err(|e| format!("could not delete {directory}: {e}"))?;
        }
    }
    deleted.sort();
    Ok(deleted)
}
