//! What the place will hold at a path given to a path macro (the TS transformer's `findSourcePath.ts`), for the
//! empty-path warnings of `finish`.
//!
//! Every disk read goes through [`Disk`]: in a build that is `FinishCx`, which records each read so that watch
//! runs `finish` again when an answer changes (the warnings then equal a clean build's); tools and tests use
//! [`FsDisk`].

use rbxts_plugin::{EntryKind, FinishCx};

use crate::node_path;

/// The reads the lookups make.
pub trait Disk {
    /// A directory's entries with whether each is a directory (links followed), sorted by name; `None` when
    /// `directory` is not a readable directory.
    fn entries(&mut self, directory: &str) -> Option<Vec<(String, bool)>>;
    /// Whether something is at `path` (links followed).
    fn exists(&mut self, path: &str) -> bool;
}

/// The disk, unrecorded.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsDisk;

impl Disk for FsDisk {
    fn entries(&mut self, directory: &str) -> Option<Vec<(String, bool)>> {
        let read = std::fs::read_dir(directory).ok()?;
        let mut entries: Vec<(String, bool)> = read
            .filter_map(Result::ok)
            .map(|entry| {
                // Follows links, as `fs.statSync` does for a symbolic link entry.
                let directory = std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir());
                (entry.file_name().to_string_lossy().into_owned(), directory)
            })
            .collect();
        entries.sort();
        Some(entries)
    }

    fn exists(&mut self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }
}

/// The compiler's recorded reads (`FinishCx::read_dir` / `exists`).
impl Disk for FinishCx<'_> {
    fn entries(&mut self, directory: &str) -> Option<Vec<(String, bool)>> {
        let entries = self.read_dir(directory)?;
        Some(entries.into_iter().map(|e| (e.name, e.kind == EntryKind::Directory)).collect())
    }

    fn exists(&mut self, path: &str) -> bool {
        FinishCx::exists(self, path)
    }
}

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

fn read_directory(disk: &mut dyn Disk, directory: &str) -> Vec<Entry> {
    disk.entries(directory)
        .unwrap_or_default()
        .into_iter()
        .map(|(name, directory)| Entry { name, directory })
        .collect()
}

/// Whether a folder holds, at any depth, a file the place gets as a module. Installed packages do not count.
fn holds_module(disk: &mut dyn Disk, directory: &str, from_source: bool) -> bool {
    read_directory(disk, directory).into_iter().any(|entry| {
        if entry.name == "node_modules" {
            return false;
        }
        if entry.directory {
            holds_module(disk, &node_path::join(&[directory, &entry.name]), from_source)
        } else {
            instance_of(&entry.name, from_source).is_some_and(|(_, module)| module)
        }
    })
}

fn find_entry(
    disk: &mut dyn Disk,
    directory: &str,
    segment: &str,
    last: bool,
    from_source: bool,
    ignore_case: bool,
) -> Option<Entry> {
    let same = |a: &str| if ignore_case { a.to_lowercase() == segment.to_lowercase() } else { a == segment };
    let entries = read_directory(disk, directory);
    let exact_index = entries.iter().position(|e| same(&e.name));
    if exact_index.is_some() || !last {
        return exact_index.map(|i| entries.into_iter().nth(i).expect("an index"));
    }
    entries.into_iter().find(|entry| {
        !entry.directory && instance_of(&entry.name, from_source).is_some_and(|(name, _)| same(&name))
    })
}

fn walk(disk: &mut dyn Disk, base: &str, segments: &[&str], from_source: bool, ignore_case: bool) -> Option<String> {
    let mut current = base.to_owned();
    for (index, segment) in segments.iter().enumerate() {
        let last = index == segments.len() - 1;
        let entry = find_entry(disk, &current, segment, last, from_source, ignore_case)?;
        if !last && !entry.directory {
            return None;
        }
        current = node_path::join(&[&current, &entry.name]);
    }
    Some(current)
}

fn state_of(disk: &mut dyn Disk, absolute: &str, from_source: bool) -> SourcePathState {
    // Not a (readable) directory: a file, which is a module.
    if disk.entries(absolute).is_none() {
        return SourcePathState::Modules;
    }
    if holds_module(disk, absolute, from_source) { SourcePathState::Modules } else { SourcePathState::Empty }
}

fn is_within(child: &str, parent: &str) -> bool {
    let relative = node_path::relative(parent, child);
    relative.is_empty() || (!relative.starts_with("..") && !node_path::is_absolute(&relative))
}

/// `findPlaceSource`: what the place holds at `rbx_path`, judged from the deepest `$path` (`partitions`:
/// `(rbx path, fs path)`) covering it; a `$path` inside `out_dir` is judged from the sources. `None` when no
/// `$path` covers it.
pub fn find_place_source(
    disk: &mut dyn Disk,
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
    if !disk.exists(&base) {
        return Some(SourcePathState::Missing(None));
    }
    let rest: Vec<&str> = rbx_path[partition_rbx.len()..].iter().map(String::as_str).collect();
    if let Some(exact) = walk(disk, &base, &rest, from_source, false) {
        return Some(state_of(disk, &exact, from_source));
    }
    Some(match walk(disk, &base, &rest, from_source, true) {
        None => SourcePathState::Missing(None),
        Some(other) => SourcePathState::Missing(Some(node_path::relative(display_base, &other).replace('\\', "/"))),
    })
}

/// `findSourcePath`: a source path looked up by itself, relative to `base`.
pub fn find_source_path(disk: &mut dyn Disk, base: &str, source_path: &str) -> SourcePathState {
    let absolute = node_path::resolve(&[base, source_path]);
    let relative = node_path::relative(base, &absolute);
    if relative.starts_with("..") || node_path::is_absolute(&relative) {
        if !disk.exists(&absolute) {
            return SourcePathState::Missing(None);
        }
        return state_of(disk, &absolute, true);
    }
    let segments: Vec<&str> = relative.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if let Some(exact) = walk(disk, base, &segments, true, false) {
        return state_of(disk, &exact, true);
    }
    match walk(disk, base, &segments, true, true) {
        None => SourcePathState::Missing(None),
        Some(other) => SourcePathState::Missing(Some(node_path::relative(base, &other).replace('\\', "/"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disk in memory, keyed by the path's tail below `/fakebase`, that remembers what was read.
    struct Fake {
        dirs: Vec<(&'static str, Vec<(&'static str, bool)>)>,
        read: Vec<String>,
    }

    impl Fake {
        fn key(path: &str) -> String {
            let path: String = path.chars().map(|c| if c == char::from(92) { '/' } else { c }).collect();
            match path.find("/fakebase") {
                Some(at) => path[at + "/fakebase".len()..].to_owned(),
                None => path,
            }
        }
    }

    impl Disk for Fake {
        fn entries(&mut self, directory: &str) -> Option<Vec<(String, bool)>> {
            let key = Fake::key(directory);
            self.read.push(key.clone());
            let (_, entries) = self.dirs.iter().find(|(d, _)| *d == key)?;
            Some(entries.iter().map(|(n, d)| (n.to_string(), *d)).collect())
        }

        fn exists(&mut self, path: &str) -> bool {
            let key = Fake::key(path);
            self.read.push(key.clone());
            self.dirs.iter().any(|(d, _)| *d == key)
        }
    }

    fn fake() -> Fake {
        Fake {
            dirs: vec![
                ("", vec![("src", true)]),
                ("/src", vec![("server", true), ("types", true), ("Glob", true)]),
                ("/src/server", vec![("main.server.ts", false)]),
                ("/src/types", vec![("a.d.ts", false)]),
                ("/src/Glob", vec![]),
            ],
            read: Vec::new(),
        }
    }

    #[test]
    fn source_paths_are_judged_through_the_disk_given() {
        let base = "/fakebase";
        let mut disk = fake();
        assert_eq!(find_source_path(&mut disk, base, "src/server"), SourcePathState::Modules);
        assert_eq!(find_source_path(&mut disk, base, "src/types"), SourcePathState::Empty);
        assert_eq!(find_source_path(&mut disk, base, "src/missing"), SourcePathState::Missing(None));
        assert_eq!(
            find_source_path(&mut disk, base, "src/glob"),
            SourcePathState::Missing(Some("src/Glob".to_owned()))
        );
        // every read went through the disk given (in a build: FinishCx, recorded)
        assert!(disk.read.contains(&"/src".to_owned()) && disk.read.contains(&"/src/server".to_owned()));
    }
}
