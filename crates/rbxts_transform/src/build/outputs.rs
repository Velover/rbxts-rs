//! The project's outputs beyond the Luau of its source files (roblox-ts's `Project/functions/getProjectOutputs.ts`):
//! the assets copied from the root directories, the set of files the clean keeps, and the clean that deletes every
//! other file under the output roots after a successful build ("stale outputs": the outputs of deleted or renamed
//! sources, and anything else roblox-ts would remove).
//!
//! The keep set follows `getProjectOutputs` rule by rule, because the clean deletes files a user may care about: it
//! must never delete a file roblox-ts keeps. Per emitted program file (not JSON): its Luau output, its
//! `.transformed.ts` with `writeTransformedFiles` and tsconfig plugins, and the `.d.ts` / `.d.ts.map` TypeScript
//! would emit for it (`ts.getOutputFileNames`, in the `declarationDir` or `outDir`). Per asset in the root
//! directories (outside the output roots and the include path, `node_modules` and `.git`; not a config, a build info
//! or a deployment `*.project.json` beside the tsconfig; `.ts` / `.tsx` only when they are declaration files of a
//! declaration build): its output. Then the build infos (roblox-ts's and tsc's own) and every Rojo project file.
//!
//! The clean, as roblox-ts's: every root is walked without following links; a directory (not a link) is cleaned
//! and removed once empty; anything else that is not kept (files, and links to files or directories, which are
//! removed themselves, never their targets) is deleted; `.git` is skipped.
//!
//! rbxts-rs: differences from roblox-ts, all keeping more or writing less:
//! - assets are written only when their bytes changed (DESIGN §14 commit step 1), not copied on every build;
//! - the include folder is kept whole when it lies under an output root (DESIGN §14: "The include folder is in the
//!   clean step's keep set"), not only the runtime files it copies ([`crate::project::copy_include`]);
//! - plugin artifacts and state files of this build are kept;
//! - declaration assets (project `.d.ts` files of a declaration build) are kept but not written: roblox-ts rewrites
//!   them (`transformDeclarationFile`, a TypeScript print), which is not ported; rbxts-rs emits no `.d.ts` either;
//! - an output whose spelling differs from the file on disk only in case is renamed to the build's spelling (a
//!   case-only rename of a source), so the Rojo instance name follows, as in a build into an empty directory.

use std::io;
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
    /// Every output path of the build (Luau, declarations, transformed files, assets, declaration assets), which
    /// plugin artifacts may not overwrite.
    pub files: Vec<String>,
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
    /// Per emitted program file: its input, its Luau output (after the override) and the declaration outputs
    /// TypeScript would emit for it (`.d.ts`, `.d.ts.map`).
    pub emitted: &'a [(String, String, Vec<String>)],
    /// `writeTransformedFiles` with tsconfig plugins: the `.transformed.ts` outputs are kept.
    pub transformed_outputs: bool,
    /// The outDir override of the build, if any.
    pub out_dir_override: Option<&'a str>,
    /// tsconfig and `extends` paths, never assets.
    pub config_paths: &'a [String],
    /// tsc's own build info (`getTsBuildInfoEmitOutputFilePath`), besides roblox-ts's `.rbxtsc.tsbuildinfo`.
    pub ts_build_info: Option<&'a str>,
}

/// Moves an output path under the override, as the Luau outputs are.
pub fn overridden(services: &ProjectServices, path: &str, out_dir_override: Option<&str>) -> String {
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
    let mut outputs = ProjectOutputs {
        roots: output_roots(services, inputs.options, inputs.out_dir_override),
        ..ProjectOutputs::default()
    };

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

    // addOutput: an output with two inputs is an error
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
        outputs.files.push(output.to_owned());
        Ok(())
    };
    for (input, luau, declarations) in inputs.emitted {
        add_output(&mut outputs, luau, input)?;
        if inputs.transformed_outputs {
            let transformed =
                overridden(services, &translator.get_output_transformed_path(input), inputs.out_dir_override);
            add_output(&mut outputs, &transformed, input)?;
        }
        for declaration in declarations {
            add_output(&mut outputs, declaration, input)?;
        }
    }

    let include = nodepath::resolve(&[&services.data.options.include_path]);
    // every output root, with or without the override (roblox-ts: `getOutputRoots` of every project)
    let mut excluded: Vec<String> = outputs.roots.iter().map(|r| path_key(services, r)).collect();
    excluded.extend(output_roots(services, inputs.options, None).iter().map(|r| path_key(services, r)));
    excluded.push(path_key(services, &include));
    let config_keys: FxHashSet<String> = inputs.config_paths.iter().map(|p| path_key(services, p)).collect();
    let build_infos: FxHashSet<String> = translator
        .build_info_output_path
        .iter()
        .map(String::as_str)
        .chain(inputs.ts_build_info)
        .map(|p| path_key(services, p))
        .collect();
    let project_dir_key = path_key(services, &services.data.project_path);

    // (path, is a directory); entries' types come from the directory listing (links are followed, as `statSync`)
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
            && base.len() > ".project.json".len()
            && base.ends_with(".project.json");
        if config_keys.contains(&key) || build_infos.contains(&key) || is_deployment_config {
            continue;
        }
        let output = overridden(services, &translator.get_output_path(&input), inputs.out_dir_override);
        if input.ends_with(".ts") || input.ends_with(".tsx") {
            if !tsrs_core::tspath::is_declaration_file_name(&input) || !translator.declaration {
                continue;
            }
            // rbxts-rs: a declaration asset is kept, not written (`transformDeclarationFile` is not ported)
            add_output(&mut outputs, &output, &input)?;
            continue;
        }
        add_output(&mut outputs, &output, &input)?;
        assets.push((input, output));
    }
    assets.sort();
    outputs.assets = assets;

    for build_info in translator.build_info_output_path.iter().map(String::as_str).chain(inputs.ts_build_info) {
        outputs.keep.insert(path_key(services, build_info));
    }
    for config in services.rojo_resolver.config_files() {
        outputs.keep.insert(path_key(services, config));
    }
    outputs.keep_dirs.push(include);
    Ok(outputs)
}

/// The assets' bytes, in `outputs.assets` order, read before the commit writes anything.
pub fn read_assets(outputs: &ProjectOutputs) -> Result<Vec<Vec<u8>>, String> {
    outputs
        .assets
        .iter()
        .map(|(input, _)| std::fs::read(input).map_err(|e| format!("could not read {input}: {e}")))
        .collect()
}

/// Commit step 1 for assets: writes each asset (`bytes` from [`read_assets`]) whose output is missing or differs.
/// rbxts-rs: an output whose spelling differs from the file on disk only in case is renamed first (see
/// [`match_case`]). Returns the outputs written.
pub fn sync_assets(
    services: &ProjectServices,
    outputs: &ProjectOutputs,
    bytes: &[Vec<u8>],
    listing: &Listing,
) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for ((_, output), bytes) in outputs.assets.iter().zip(bytes) {
        match_case(services, output, listing).map_err(|e| format!("could not rename to {output}: {e}"))?;
        if write_if_changed(Path::new(output), bytes).map_err(|e| format!("could not write {output}: {e}"))? {
            written.push(output.clone());
        }
    }
    Ok(written)
}

/// The files under the output roots before a build writes anything, taken once per build: for the missing-output
/// check, the case check and the clean. The include folder and `.git` are not entered, nor links.
#[derive(Debug, Default)]
pub struct Listing {
    /// (path, key) of every file, and of every link (to a file or a directory).
    pub files: Vec<(String, String)>,
    /// Path key → the path as spelled on disk.
    pub spelled: FxHashMap<String, String>,
    /// The links among `files` (removed as links, never followed).
    pub links: FxHashSet<String>,
    /// The directories that were empty (roblox-ts's clean removes empty directories too).
    pub empty_dirs: Vec<String>,
    /// The roots listed, as spelled.
    pub roots: Vec<String>,
}

impl Listing {
    /// A file with this key was there.
    pub fn contains(&self, key: &str) -> bool {
        self.spelled.contains_key(key)
    }
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
        let declaration_dir = nodepath::resolve(&[&services.data.project_path, &options.declaration_dir]);
        if !roots.iter().any(|r| path_key(services, r) == path_key(services, &declaration_dir)) {
            roots.push(declaration_dir);
        }
    }
    roots
}

/// Lists the files under `roots`.
pub fn list(services: &ProjectServices, roots: &[String]) -> Listing {
    let include = path_key(services, &nodepath::resolve(&[&services.data.options.include_path]));
    let mut listing = Listing { roots: roots.to_vec(), ..Listing::default() };
    // a root that is a link is not entered (roblox-ts's `existsSync` + `readdirSync` would follow it; its clean of
    // the link's contents would delete files outside the project)
    let mut stack: Vec<String> = roots
        .iter()
        .filter(|r| std::fs::symlink_metadata(r.as_str()).is_ok_and(|m| m.file_type().is_dir()))
        .cloned()
        .collect();
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
            if file_type.is_dir() && !file_type.is_symlink() {
                if !is_descendant_key(&key, &include) {
                    stack.push(path);
                }
            } else {
                if file_type.is_symlink() {
                    listing.links.insert(key.clone());
                }
                listing.spelled.insert(key.clone(), path.clone());
                listing.files.push((path, key));
            }
        }
        if count == 0 {
            listing.empty_dirs.push(directory);
        }
    }
    listing
}

/// rbxts-rs: a deliberate deviation from roblox-ts, which writes into the existing file and keeps its old name, so
/// after a case-only rename of a source the Rojo instance keeps the old spelling (a roblox-ts bug).
///
/// When the listing has `path` spelled differently (only in case), renames the differing components below the
/// output root on disk to `path`'s spelling, from the root down, so a build into a used directory names its outputs
/// as a build into an empty one does. Nothing above the output root is touched.
pub fn match_case(services: &ProjectServices, path: &str, listing: &Listing) -> io::Result<()> {
    let key = path_key(services, path);
    let Some(existing) = listing.spelled.get(&key) else { return Ok(()) };
    let wanted = nodepath::normalize(path).replace('\\', "/");
    if existing.replace('\\', "/") == wanted {
        return Ok(());
    }
    let Some(root) = listing.roots.iter().find(|r| is_descendant_key(&key, &path_key(services, r))) else {
        return Ok(());
    };
    let root = nodepath::normalize(root).replace('\\', "/");
    let root = root.trim_end_matches('/');
    let Some(below) = wanted.get(root.len()..).map(|rest| rest.trim_start_matches('/')) else { return Ok(()) };
    let mut directory = root.to_owned();
    for part in below.split('/') {
        let next = format!("{directory}/{part}");
        let on_disk = std::fs::read_dir(&directory)?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .find(|name| name.to_lowercase() == part.to_lowercase());
        if let Some(on_disk) = on_disk
            && on_disk != part
        {
            std::fs::rename(format!("{directory}/{on_disk}"), &next)?;
        }
        directory = next;
    }
    Ok(())
}

/// Commit step 3 for stale outputs: deletes every listed file and link that is not kept (a link itself, never what
/// it points to), then the directories that leaves empty and those that were empty (never a root). Returns the
/// files deleted.
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
        let removed =
            if listing.links.contains(key) { remove_link(Path::new(path)) } else { remove_file(Path::new(path)) };
        if removed.map_err(|e| format!("could not delete {path}: {e}"))? {
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

/// Removes a link itself: a file link with `remove_file`, a directory link (a junction or a directory symlink) with
/// `remove_dir`, which on a link removes the link and leaves its target alone.
fn remove_link(path: &Path) -> io::Result<bool> {
    match remove_file(path) {
        Ok(removed) => Ok(removed),
        Err(_) => std::fs::remove_dir(path).map(|()| true),
    }
}
