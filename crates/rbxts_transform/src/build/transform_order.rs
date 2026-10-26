//! rbxts-rs only: the order roblox-ts transforms a clean build's files in, `getChangedFilePaths` on a fresh builder
//! (`Project/functions/getChangedFilePaths.ts`, TypeScript 5's `BuilderState`).
//!
//! roblox-ts transforms its files, and runs the TS transformer plugins over them, in the order of the changed-file set:
//! each file of the builder's `changedFilesSet` (a fresh builder's holds every program file, in program order) is
//! searched depth first through the reversed `referencedMap` (the files that import it, in program order), adding
//! every file met. So a file is followed at once by its importers, transitively, and that order is visible: TS5
//! Flamework allocates its declaration ids, and fills `flamework.build`'s identifier table, in it. rbxts-rs makes it
//! the order of a build's files (transform, reports, [`rbxts_plugin::FileIndex`]); every build of a program uses the
//! clean build's order, so a watch rebuild or an incremental build orders its files as a clean build does. It is a
//! function of the program, the same for any checker count.
//!
//! `referencedMap` is TypeScript 5's `getReferencedFiles`, rebuilt from the program's resolutions instead of the
//! checker (`getSymbolAtLocation` of each import's module name gives the declaring files of the module symbol):
//! - each resolved import and module augmentation (type-only imports, `export … from`, `import()` and `import("…")`
//!   types included) references the module's file and every file that augments that module;
//! - an import of an ambient module (`declare module "name"` in a script file) references the files declaring it and
//!   the module files augmenting it (`declare module "name"` in a module file, a name that resolves to no file);
//! - triple-slash path references (the path as written, resolved against the file's directory) and the file's
//!   resolved type reference directives;
//! - every file references the files declaring an ambient module with more than one declaration (other than itself).
//!
//! Not modelled: pattern ambient modules (`declare module "*.png"`) and module names only the checker resolves. A
//! non-relative import that both resolves to a file and matches a `declare module "name"` is the ambient module to
//! TypeScript 5's checker (so it references the declaring files), while this references the file it resolves to;
//! reachable only when a project shadows an installed package with an ambient declaration.

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::SourceFile;
use tsrs_compiler::Program;
use tsrs_core::P;
use tsrs_core::tspath;

/// Every program file (declaration files included), in roblox-ts's clean-build transform order.
pub fn transform_order(program: &Program) -> Vec<P<SourceFile>> {
    let files = program.source_files();
    let cwd = program.get_current_directory();
    let case_sensitive = program.use_case_sensitive_file_names();
    let key = |name: &str| tspath::to_path(name, cwd, case_sensitive).to_string();
    let index: FxHashMap<String, usize> = files.iter().enumerate().map(|(i, f)| (f.path().to_string(), i)).collect();
    let resolved_modules = program.get_resolved_modules();
    let type_references = program.get_resolved_type_reference_directives();

    // the modules each file augments (by the module's file, or by name when the name resolves to no file: an
    // ambient module's augmentation), and the files declaring each ambient module (one entry per declaration)
    let mut augmenters: FxHashMap<String, Vec<usize>> = FxHashMap::default();
    let mut ambient_augmenters: FxHashMap<&str, Vec<usize>> = FxHashMap::default();
    let mut ambient: FxHashMap<&str, Vec<usize>> = FxHashMap::default();
    for (i, &file) in files.iter().enumerate() {
        for &name in file.ambient_module_names() {
            ambient.entry(name).or_default().push(i);
        }
        let augmentations: FxHashSet<&str> = file
            .module_augmentations()
            .iter()
            .filter(|&&name| tsrs_ast::is_string_literal(name))
            .map(|name| name.text())
            .collect();
        if augmentations.is_empty() {
            continue;
        }
        let mut to_files: FxHashSet<&str> = FxHashSet::default();
        if let Some(resolutions) = resolved_modules.get(file.path()) {
            for (cache_key, resolved) in resolutions {
                if augmentations.contains(cache_key.name) && resolved.is_resolved() {
                    augmenters.entry(key(resolved.resolved_file_name)).or_default().push(i);
                    to_files.insert(cache_key.name);
                }
            }
        }
        for &name in augmentations.iter().filter(|name| !to_files.contains(*name)) {
            ambient_augmenters.entry(name).or_default().push(i);
        }
    }
    // `getAmbientModules()` with more than one declaration: every file references their declaring files
    let mut everyone: Vec<usize> =
        ambient.values().filter(|declaring| declaring.len() > 1).flatten().copied().collect();
    everyone.sort_unstable();
    everyone.dedup();

    // the reversed referencedMap: per referenced file, the files referencing it, in program order
    let mut importers: Vec<Vec<usize>> = vec![Vec::new(); files.len()];
    let mut referenced: FxHashSet<usize> = FxHashSet::default();
    for (i, &file) in files.iter().enumerate() {
        referenced.clear();
        let add_key = |referenced: &mut FxHashSet<usize>, path: &str| {
            if let Some(&target) = index.get(path) {
                referenced.insert(target);
            }
        };
        if let Some(resolutions) = resolved_modules.get(file.path()) {
            for (cache_key, resolved) in resolutions {
                if resolved.is_resolved() {
                    let module = key(resolved.resolved_file_name);
                    add_key(&mut referenced, &module);
                    if let Some(augmenting) = augmenters.get(&module) {
                        referenced.extend(augmenting.iter().copied());
                    }
                } else if let Some(declaring) = ambient.get(cache_key.name) {
                    // TS5's `getSymbolAtLocation` gives the merged ambient module: its declarations and the module
                    // files augmenting it (they don't count in `getAmbientModules`, whose symbol is the unmerged one)
                    referenced.extend(declaring.iter().copied());
                    if let Some(augmenting) = ambient_augmenters.get(cache_key.name) {
                        referenced.extend(augmenting.iter().copied());
                    }
                }
            }
        }
        let directory = tspath::get_directory_path(file.file_name());
        for reference in file.referenced_files() {
            add_key(&mut referenced, tspath::to_path(&reference.file_name, &directory, case_sensitive).as_str());
        }
        if let Some(directives) = type_references.get(file.path()) {
            for resolved in directives.values() {
                if !resolved.resolved_file_name.is_empty() {
                    add_key(&mut referenced, &key(resolved.resolved_file_name));
                }
            }
        }
        referenced.extend(everyone.iter().copied().filter(|&d| d != i));
        for &target in &referenced {
            importers[target].push(i);
        }
    }

    // `search`, depth first from each file in program order; a file joins the order when first met
    let mut seen = vec![false; files.len()];
    let mut order = Vec::with_capacity(files.len());
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for start in 0..files.len() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        order.push(files[start]);
        stack.push((start, 0));
        while let Some((file, next)) = stack.last_mut() {
            match importers[*file].get(*next) {
                Some(&importer) => {
                    *next += 1;
                    if !seen[importer] {
                        seen[importer] = true;
                        order.push(files[importer]);
                        stack.push((importer, 0));
                    }
                }
                None => {
                    stack.pop();
                }
            }
        }
    }
    order
}
