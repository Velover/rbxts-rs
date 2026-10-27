//! Parsed source files kept across the builds of one process (watch mode, DESIGN §14): a compiler host that hands
//! out the same `SourceFile` for a file whose stamp ([`crate::stamp`]: modification and change times and size, asked of
//! the file) and parse options are unchanged, as roblox-ts's program factory does (an unchanged file is the same object
//! in the next program) and as tsrs's own watcher does. A stamp taken too soon after the file changed to tell a later
//! edit apart (not settled) serves the file only when its text is the same.
//!
//! The files outlive the programs that use them: a watch session frees each program once a later one replaces it
//! ([`crate::Project::free`]), and the files, bound before any checker existed, go on in the next program. A file's
//! older versions are never freed (a kept diagnostic may still name one).

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex};

use tsrs_ast::{SourceFile, SourceFileParseOptions};
use tsrs_compiler::CompilerHost;
use tsrs_core::P;
use tsrs_core::tspath::Path;
use tsrs_diagnostics::Message;
use tsrs_tsoptions::ParsedCommandLine;
use tsrs_vfs::FS;

use crate::read_log::{ReadLog, Seen};
use crate::stamp::{self, FileStamp, PathStat};

struct Entry {
    file: P<SourceFile>,
    options: SourceFileParseOptions,
    stamp: FileStamp,
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

    /// How many files the hosts over this cache parsed (cache misses), for timings and tests.
    pub fn parses(&self) -> usize {
        self.parses.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The stamp a file had when the cache last served or parsed it: what the last program that loaded it read (a
    /// watcher compares it with the disk).
    pub fn stamp(&self, path: &Path) -> Option<FileStamp> {
        self.files.lock().expect("source file cache").get(path).map(|e| e.stamp)
    }
}

/// A compiler host over another one that serves unchanged files from a [`SourceFileCache`].
pub(crate) struct CachingHost {
    pub inner: Arc<dyn CompilerHost>,
    pub cache: Arc<SourceFileCache>,
    /// The load's read log: the stamp's question is a read (the file's kind).
    pub reads: Option<Arc<ReadLog>>,
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
        let stamp = if tsrs_vfs::bundled::embedded_file(&opts.file_name).is_some() {
            Some(FileStamp::FIXED)
        } else {
            let stat = stamp::stat(&opts.file_name);
            if let Some(log) = &self.reads {
                log.see(&opts.file_name, match stat {
                    PathStat::Missing => Seen::NO_FILE | Seen::NO_DIR,
                    PathStat::Dir => Seen::DIR,
                    PathStat::File(_) => Seen::FILE,
                });
            }
            stat.file()
        };
        if let Some(stamp) = stamp {
            let cached = {
                let files = self.cache.files.lock().expect("source file cache");
                files
                    .get(&opts.path)
                    .filter(|entry| entry.options == opts && entry.stamp.same(&stamp))
                    .map(|entry| (entry.file, entry.stamp.settled))
            };
            if let Some((file, settled)) = cached {
                // a stamp too close to the change it saw: the text tells (and the new stamp may be settled)
                if settled || self.inner.fs().read_file(&opts.file_name).is_some_and(|text| text == file.text()) {
                    if !settled {
                        let mut files = self.cache.files.lock().expect("source file cache");
                        if let Some(entry) = files.get_mut(&opts.path) {
                            entry.stamp = stamp;
                        }
                    }
                    return Some(file);
                }
            }
        }
        let path = opts.path.clone();
        let parsed = self.inner.get_source_file(opts.clone());
        self.cache.parses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut files = self.cache.files.lock().expect("source file cache");
        match (parsed, stamp) {
            (Some(file), Some(stamp)) => {
                files.insert(path, Entry { file, options: opts, stamp });
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
