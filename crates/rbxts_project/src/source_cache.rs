//! Parsed source files kept across the builds of one process (watch mode, DESIGN §14): a compiler host that hands
//! out the same `SourceFile` for a file whose modification time, size and parse options are unchanged, as roblox-ts's
//! program factory does (an unchanged file is the same object in the next program) and as tsrs's own watcher does.
//!
//! The files are never freed: freeing a program needs tsrs's `free_program`, an `unsafe fn` (rbxts-rs has no
//! `unsafe`). The watcher restarts its process past a memory limit instead (DESIGN §14 "Memory").

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tsrs_ast::{SourceFile, SourceFileParseOptions};
use tsrs_compiler::CompilerHost;
use tsrs_core::P;
use tsrs_core::tspath::Path;
use tsrs_diagnostics::Message;
use tsrs_tsoptions::ParsedCommandLine;
use tsrs_vfs::FS;

struct Entry {
    file: P<SourceFile>,
    options: SourceFileParseOptions,
    mod_time: Option<SystemTime>,
    size: i64,
}

/// The parsed files of a process's builds, by path. Shared by every program the process makes for one project.
#[derive(Default)]
pub struct SourceFileCache {
    files: Mutex<HashMap<Path, Entry>>,
    /// Files parsed (not served from the cache) since the cache was made.
    parses: std::sync::atomic::AtomicUsize,
}

impl SourceFileCache {
    /// An empty cache.
    pub fn new() -> SourceFileCache {
        SourceFileCache::default()
    }

    /// The files among `files` (path and file name) that changed on disk since the cache parsed them: another
    /// modification time or size, or gone. Files the cache never parsed are left out. For files no watcher watches
    /// (packages), checked before a program is updated in place.
    pub fn changed_on_disk<'a>(&self, files: impl IntoIterator<Item = (&'a Path, &'a str)>) -> Vec<String> {
        let cached = self.files.lock().expect("source file cache");
        let mut changed = Vec::new();
        for (path, file_name) in files {
            let Some(entry) = cached.get(path) else { continue };
            let same = std::fs::metadata(file_name)
                .is_ok_and(|m| m.modified().ok() == entry.mod_time && m.len() as i64 == entry.size);
            if !same {
                changed.push(file_name.to_owned());
            }
        }
        changed
    }

    /// How many files the hosts over this cache parsed (cache misses), for timings and tests.
    pub fn parses(&self) -> usize {
        self.parses.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// A compiler host over another one that serves unchanged files from a [`SourceFileCache`].
pub(crate) struct CachingHost {
    pub inner: Arc<dyn CompilerHost>,
    pub cache: Arc<SourceFileCache>,
}

impl CompilerHost for CachingHost {
    fn fs(&self) -> &dyn FS {
        self.inner.fs()
    }

    fn default_library_path(&self) -> &str {
        self.inner.default_library_path()
    }

    fn get_current_directory(&self) -> &str {
        self.inner.get_current_directory()
    }

    fn trace(&self, msg: &'static Message, args: &[&dyn Display]) {
        self.inner.trace(msg, args);
    }

    fn get_source_file(&self, opts: SourceFileParseOptions) -> Option<P<SourceFile>> {
        let info = self.inner.fs().stat(&opts.file_name);
        if let Some(info) = &info {
            let files = self.cache.files.lock().expect("source file cache");
            if let Some(entry) = files.get(&opts.path)
                && entry.options == opts
                && entry.mod_time == info.mod_time()
                && entry.size == info.size()
            {
                return Some(entry.file);
            }
        }
        let path = opts.path.clone();
        let parsed = self.inner.get_source_file(opts.clone());
        self.cache.parses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut files = self.cache.files.lock().expect("source file cache");
        match (parsed, info) {
            (Some(file), Some(info)) => {
                files.insert(path, Entry { file, options: opts, mod_time: info.mod_time(), size: info.size() });
            }
            _ => {
                files.remove(&path);
            }
        }
        parsed
    }

    fn get_content_mapped_source_files(
        &self,
        parse_options: SourceFileParseOptions,
        mapper: &tsrs_tsoptions::Mapper,
    ) -> Result<tsrs_contentmapper::SourceFiles, tsrs_contentmapper::Error> {
        self.inner.get_content_mapped_source_files(parse_options, mapper)
    }

    fn content_mapper_project(&self) -> Option<Arc<dyn tsrs_contentmapper::Project>> {
        self.inner.content_mapper_project()
    }

    fn get_resolved_project_reference(&self, file_name: &str, path: Path) -> Option<P<ParsedCommandLine>> {
        self.inner.get_resolved_project_reference(file_name, path)
    }
}
