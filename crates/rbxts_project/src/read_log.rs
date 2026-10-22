//! What a program load read from the file system (watch mode, DESIGN §14 "Watcher"): every path the tsconfig parse
//! and the program asked about, with what they learned, as tsrs's own watch mode tracks it (`trackingvfs`, Go's
//! `SeenFiles`). Paths that did not exist count as much as files read: they are the places module resolution
//! probed (`node_modules/<name>` in every ancestor, `@types`, `typeRoots`, `paths`), where a file appearing changes
//! the program.
//!
//! JSON files read (package.json, configs) also keep a hash of their text: a rewrite with the same bytes changes
//! nothing the load saw.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use tsrs_vfs::{Entries, FS, FileInfo};

/// What reads of one path learned (several reads of a path add up).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Seen(u8);

impl Seen {
    /// A file was there (a read or a positive `file_exists` / `stat`).
    pub const FILE: Seen = Seen(1);
    /// A directory was there.
    pub const DIR: Seen = Seen(2);
    /// `file_exists` said no: no file there (maybe a directory).
    pub const NO_FILE: Seen = Seen(4);
    /// `directory_exists` said no: no directory there (maybe a file).
    pub const NO_DIR: Seen = Seen(8);
    /// The directory's entries were listed.
    pub const LISTED: Seen = Seen(16);

    /// Whether every flag of `other` is set.
    pub fn has(self, other: Seen) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether the path was there when read.
    pub fn existed(self) -> bool {
        self.0 & (Seen::FILE.0 | Seen::DIR.0) != 0
    }

    /// Whether what is on disk now (`is_file`, `is_dir`) contradicts what was read.
    pub fn contradicted(self, is_file: bool, is_dir: bool) -> bool {
        (self.has(Seen::FILE) && !is_file)
            || (self.has(Seen::DIR) && !is_dir)
            || (self.has(Seen::NO_FILE) && is_file)
            || (self.has(Seen::NO_DIR) && is_dir)
    }
}

impl std::ops::BitOr for Seen {
    type Output = Seen;
    fn bitor(self, other: Seen) -> Seen {
        Seen(self.0 | other.0)
    }
}

/// The paths one load read, by their spelling in the reads.
#[derive(Default)]
pub struct ReadLog {
    paths: Mutex<HashMap<String, Seen>>,
    /// [`text_hash`] of the JSON files read, by their spelling in the reads.
    texts: Mutex<HashMap<String, u64>>,
}

/// The hash a read log keeps of a JSON file's text (FNV-1a over the UTF-8 bytes, a leading byte-order mark left
/// out): equal hashes of a file's text now and as read mean the same text.
pub fn text_hash(text: &str) -> u64 {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash ^ (text.len() as u64).rotate_left(32)
}

impl ReadLog {
    /// Records what a read of `path` learned (setup's reads outside the program's file system go here too).
    pub fn see(&self, path: &str, seen: Seen) {
        // the bundled lib files are not on disk
        if path.starts_with("bundled:") {
            return;
        }
        let mut paths = self.paths.lock().expect("read log");
        match paths.get_mut(path) {
            Some(entry) => *entry = *entry | seen,
            None => {
                paths.insert(path.to_owned(), seen);
            }
        }
    }

    /// Records a file read: there with `text`, or not there. A JSON file's text is hashed ([`text_hash`]).
    pub fn see_text(&self, path: &str, text: Option<&str>) {
        self.see(path, if text.is_some() { Seen::FILE } else { Seen::NO_FILE });
        if let Some(text) = text
            && path.ends_with(".json")
        {
            self.texts.lock().expect("read log").insert(path.to_owned(), text_hash(text));
        }
    }

    /// The JSON files read with the hash of their text (unordered).
    pub fn texts(&self) -> Vec<(String, u64)> {
        self.texts.lock().expect("read log").iter().map(|(p, h)| (p.clone(), *h)).collect()
    }

    /// Every path read, with what was learned (unordered).
    pub fn entries(&self) -> Vec<(String, Seen)> {
        self.paths.lock().expect("read log").iter().map(|(p, s)| (p.clone(), *s)).collect()
    }

    /// How many paths were read.
    pub fn len(&self) -> usize {
        self.paths.lock().expect("read log").len()
    }

    /// Whether nothing was read.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A file system that records reads in a [`ReadLog`]. Writes are not recorded.
pub(crate) struct RecordingFs<T: FS> {
    pub inner: T,
    pub log: std::sync::Arc<ReadLog>,
}

impl<T: FS> FS for RecordingFs<T> {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.inner.use_case_sensitive_file_names()
    }

    fn file_exists(&self, path: &str) -> bool {
        let exists = self.inner.file_exists(path);
        self.log.see(path, if exists { Seen::FILE } else { Seen::NO_FILE });
        exists
    }

    fn read_file(&self, path: &str) -> Option<String> {
        let text = self.inner.read_file(path);
        self.log.see_text(path, text.as_deref());
        text
    }

    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.inner.write_file(path, data)
    }

    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.inner.append_file(path, data)
    }

    fn remove(&self, path: &str) -> Result<(), String> {
        self.inner.remove(path)
    }

    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        self.inner.chtimes(path, a_time, m_time)
    }

    fn directory_exists(&self, path: &str) -> bool {
        let exists = self.inner.directory_exists(path);
        self.log.see(path, if exists { Seen::DIR } else { Seen::NO_DIR });
        exists
    }

    fn get_accessible_entries(&self, path: &str) -> Entries {
        let entries = self.inner.get_accessible_entries(path);
        self.log.see(path, Seen::LISTED);
        entries
    }

    fn stat(&self, path: &str) -> Option<FileInfo> {
        let info = self.inner.stat(path);
        let seen = match &info {
            Some(info) if info.is_dir() => Seen::DIR,
            Some(_) => Seen::FILE,
            None => Seen::NO_FILE | Seen::NO_DIR,
        };
        self.log.see(path, seen);
        info
    }

    fn realpath(&self, path: &str) -> String {
        self.inner.realpath(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{Seen, text_hash};

    #[test]
    fn contradictions() {
        assert!(Seen::NO_FILE.contradicted(true, false));
        assert!(!Seen::NO_FILE.contradicted(false, true));
        assert!((Seen::NO_FILE | Seen::NO_DIR).contradicted(false, true));
        assert!(Seen::FILE.contradicted(false, false));
        assert!(!Seen::DIR.contradicted(false, true));
        assert!(!Seen::LISTED.contradicted(false, false));
    }

    #[test]
    fn text_hashes() {
        assert_eq!(text_hash("{}"), text_hash("\u{feff}{}"));
        assert_ne!(text_hash("{}"), text_hash("{ }"));
        assert_ne!(text_hash(""), text_hash("\0"));
    }
}
