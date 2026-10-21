//! What the place will hold at a path given to a path macro (the TS transformer's `findSourcePath.ts`), for the
//! empty-path warnings of `finish`.

use std::path::Path;

use crate::node_path;

/// What the place holds at a path: a module (or modules under a folder), an empty folder, or nothing by that exact
/// name (with what is there when only the case differs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourcePathState {
    Modules,
    Empty,
    Missing(Option<String>),
}

/// The files Rojo makes an instance of, by the extension stripped for its name, and whether it is or may hold a
/// module. `.meta.json` makes none.
const ROJO_FILES: [(&str, bool); 12] = [
    (".model.json", true),
    (".project.json", true),
    (".luau", true),
    (".lua", true),
    (".json", true),
    (".toml", true),
    (".yaml", true),
    (".yml", true),
    (".rbxm", true),
    (".rbxmx", true),
    (".txt", false),
    (".csv", false),
];

/// A script's name without `.server` / `.client`.
fn strip_script_kind(name: &str) -> &str {
    name.strip_suffix(".server").or_else(|| name.strip_suffix(".client")).unwrap_or(name)
}

/// The instance a file becomes, and whether it is a module.
fn instance_of(file_name: &str, from_source: bool) -> Option<(String, bool)> {
    if file_name.ends_with(".meta.json") {
        return None;
    }
    if from_source {
        if file_name.ends_with(".d.ts") {
            return None;
        }
        for extension in [".tsx", ".ts"] {
            if let Some(stem) = file_name.strip_suffix(extension) {
                let name = strip_script_kind(stem);
                return Some((if name == "index" { "init".to_owned() } else { name.to_owned() }, true));
            }
        }
    }
    let (extension, module) = ROJO_FILES.iter().find(|(e, _)| file_name.ends_with(e))?;
    Some((strip_script_kind(&file_name[..file_name.len() - extension.len()]).to_owned(), *module))
}

struct Entry {
    name: String,
    directory: bool,
}

fn read_directory(directory: &str) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(directory) else { return Vec::new() };
    read.filter_map(Result::ok)
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Follows links, as `fs.statSync` does for a symbolic link entry.
            let directory = std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir());
            Entry { name, directory }
        })
        .collect()
}

/// Whether a folder holds, at any depth, a file the place gets as a module. Installed packages do not count.
fn holds_module(directory: &str, from_source: bool) -> bool {
    read_directory(directory).into_iter().any(|entry| {
        if entry.name == "node_modules" {
            return false;
        }
        if entry.directory {
            holds_module(&node_path::join(&[directory, &entry.name]), from_source)
        } else {
            instance_of(&entry.name, from_source).is_some_and(|(_, module)| module)
        }
    })
}

fn find_entry(directory: &str, segment: &str, last: bool, from_source: bool, ignore_case: bool) -> Option<Entry> {
    let same = |a: &str| if ignore_case { a.to_lowercase() == segment.to_lowercase() } else { a == segment };
    let entries = read_directory(directory);
    let exact_index = entries.iter().position(|e| same(&e.name));
    if exact_index.is_some() || !last {
        return exact_index.map(|i| entries.into_iter().nth(i).expect("an index"));
    }
    entries.into_iter().find(|entry| {
        !entry.directory && instance_of(&entry.name, from_source).is_some_and(|(name, _)| same(&name))
    })
}

fn walk(base: &str, segments: &[&str], from_source: bool, ignore_case: bool) -> Option<String> {
    let mut current = base.to_owned();
    for (index, segment) in segments.iter().enumerate() {
        let last = index == segments.len() - 1;
        let entry = find_entry(&current, segment, last, from_source, ignore_case)?;
        if !last && !entry.directory {
            return None;
        }
        current = node_path::join(&[&current, &entry.name]);
    }
    Some(current)
}

fn state_of(absolute: &str, from_source: bool) -> SourcePathState {
    if !Path::new(absolute).is_dir() {
        return SourcePathState::Modules;
    }
    if holds_module(absolute, from_source) { SourcePathState::Modules } else { SourcePathState::Empty }
}

fn is_within(child: &str, parent: &str) -> bool {
    let relative = node_path::relative(parent, child);
    relative.is_empty() || (!relative.starts_with("..") && !node_path::is_absolute(&relative))
}

/// `findPlaceSource`: what the place holds at `rbx_path`, judged from the deepest `$path` (`partitions`:
/// `(rbx path, fs path)`) covering it; a `$path` inside `out_dir` is judged from the sources. `None` when no
/// `$path` covers it.
pub fn find_place_source(
    rbx_path: &[String],
    partitions: &[(Vec<String>, String)],
    root_dir: &str,
    out_dir: &str,
    display_base: &str,
) -> Option<SourcePathState> {
    let mut partition: Option<&(Vec<String>, String)> = None;
    for candidate in partitions {
        let covers = candidate.0.len() <= rbx_path.len() && candidate.0.iter().zip(rbx_path).all(|(a, b)| a == b);
        if covers && partition.is_none_or(|p| candidate.0.len() > p.0.len()) {
            partition = Some(candidate);
        }
    }
    let (partition_rbx, partition_fs) = partition?;
    let from_source = is_within(partition_fs, out_dir);
    let base = if from_source {
        node_path::join(&[root_dir, &node_path::relative(out_dir, partition_fs)])
    } else {
        partition_fs.clone()
    };
    if !Path::new(&base).exists() {
        return Some(SourcePathState::Missing(None));
    }
    let rest: Vec<&str> = rbx_path[partition_rbx.len()..].iter().map(String::as_str).collect();
    if let Some(exact) = walk(&base, &rest, from_source, false) {
        return Some(state_of(&exact, from_source));
    }
    Some(match walk(&base, &rest, from_source, true) {
        None => SourcePathState::Missing(None),
        Some(other) => SourcePathState::Missing(Some(node_path::relative(display_base, &other).replace('\\', "/"))),
    })
}

/// `findSourcePath`: a source path looked up by itself, relative to `base`.
pub fn find_source_path(base: &str, source_path: &str) -> SourcePathState {
    let absolute = node_path::resolve(&[base, source_path]);
    let relative = node_path::relative(base, &absolute);
    if relative.starts_with("..") || node_path::is_absolute(&relative) {
        if !Path::new(&absolute).exists() {
            return SourcePathState::Missing(None);
        }
        return state_of(&absolute, true);
    }
    let segments: Vec<&str> = relative.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if let Some(exact) = walk(base, &segments, true, false) {
        return state_of(&exact, true);
    }
    match walk(base, &segments, true, true) {
        None => SourcePathState::Missing(None),
        Some(other) => SourcePathState::Missing(Some(node_path::relative(base, &other).replace('\\', "/"))),
    }
}
