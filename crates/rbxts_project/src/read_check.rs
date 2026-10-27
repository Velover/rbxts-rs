//! Whether what a load read ([`ReadLog`]) still reads the same: the check a watch rebuild makes before it reuses the
//! last load (its tsconfig parse, module resolution and project setup), instead of loading again.
//!
//! The check asks every question of the log again and compares the answers: each path's existence and kind, the text
//! of each file read, each directory listing as its reader saw it ([`ListingView`]), each `realpath`. The program's
//! files are compared by the stamps their parsed versions have in the source cache ([`crate::stamp`], asked of each
//! file), as a load compares them: the ones that differ, or whose stamp was not settled, are the files to parse again
//! (the source cache serves an unsettled one whose text is the same), and the rest of the load stands.
//!
//! **One listing per directory.** A file system query costs about as much as a directory listing on Windows (a
//! handle opened per path, through the virus scanner), and the log holds thousands of paths (module resolution's
//! probes), so the check lists the directory of each path once, in parallel, and answers every question about the
//! directory's entries from that listing: about ten times fewer system calls than the load made, which asked them one
//! at a time. A link's target is looked up on its own (its kind is the target's, as the load saw it). A listing
//! answers only names and kinds: the times and sizes it shows are copies the directory keeps, which lag behind a file
//! a writer holds open ([`crate::stamp`]).
//!
//! **Texts.** A file read as text (a `package.json`, the tsconfig, a Rojo project) is compared by its stamp, taken
//! before the read when settled ([`crate::read_log::stamp_before_read`]): the same stamp, change time included, is the
//! same text; another stamp, or none kept, and the text is read again and compared.
//!
//! **One pass.** A load keeps three logs (its tsconfig parse, its program, its setup: [`ReadLog::recheck_all`]), which
//! ask about mostly the same directories: every question of the three is asked in one parallel pass, each directory
//! listed once.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::read_log::{EntryKind, ListingView, ReadLog, Seen, listing_hash, text_hash};
use crate::stamp::{self, FileStamp, PathStat};

/// How close to now a modification time must not be for a stamp to rely on it: an edit within the same tick of the
/// file system's clock would get the same time (git's "racily clean" entries). The one place the rule lives: the
/// program files' stamps ([`crate::stamp`]) and the output stamps (`rbxts_transform`'s `plugins::lifecycle`) use
/// [`settled_time`], so such a stamp is not relied on and the next check compares the text or the bytes.
pub const RACY: Duration = Duration::from_secs(2);

/// `modified` (nanoseconds since the Unix epoch) when it is far enough in the past to tell a later edit apart, else 0
/// (no time to rely on).
pub fn settled_time(modified: u128) -> u128 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    if modified != 0 && now >= modified + RACY.as_nanos() { modified } else { 0 }
}

/// A check's answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Recheck {
    /// Everything the load read reads the same, but for these program files (by file name), whose modification time
    /// or size changed since they were parsed.
    Same {
        /// The program files to parse again.
        changed: Vec<String>,
    },
    /// Something reads differently: what, for debugging.
    Different(String),
}

/// One directory entry, as the check sees it.
#[derive(Clone, Debug)]
struct Entry {
    name: String,
    /// Links followed; `Other` for a broken link (with `exists` false).
    kind: EntryKind,
    exists: bool,
    link: bool,
}

/// One directory's listing: `None` when it could not be listed (missing, or not a directory).
type View = Option<HashMap<String, Entry>>;

/// A program file's stamp in the source cache: its file name, and its stamp when last served.
pub type ProgramStamp = (String, Option<FileStamp>);

/// The program file stamps the source cache has.
pub type ProgramStamps<'a> = &'a [ProgramStamp];

/// A path's directory and name, separators either way (a root has no name: `None`).
fn split(path: &str) -> Option<(&str, &str)> {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let at = trimmed.rfind(['/', '\\'])?;
    let (dir, name) = (&trimmed[..at], &trimmed[at + 1..]);
    if name.is_empty() {
        return None;
    }
    // `E:/a` is in `E:/`, not `E:`
    let dir = if dir.ends_with(':') || dir.is_empty() { &trimmed[..=at] } else { dir };
    Some((dir, name))
}

/// The key a directory or entry name is looked up by.
fn key(text: &str, case_sensitive: bool) -> String {
    let text = text.replace('\\', "/");
    let text = text.trim_end_matches('/');
    let text = if text.ends_with(':') { format!("{text}/") } else { text.to_owned() };
    if case_sensitive { text } else { text.to_lowercase() }
}

/// Lists a directory as the check sees it.
fn list(dir: &str, case_sensitive: bool) -> View {
    let read = std::fs::read_dir(dir).ok()?;
    let mut entries = HashMap::new();
    for entry in read {
        let Ok(entry) = entry else { return None };
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(own) = entry.file_type() else { continue };
        let link = own.is_symlink();
        let kind = if link { std::fs::metadata(entry.path()).map(|m| m.file_type()) } else { Ok(own) };
        let (kind, exists) = match kind {
            Ok(t) if t.is_dir() => (EntryKind::Dir, true),
            Ok(t) if t.is_file() => (EntryKind::File, true),
            Ok(_) => (EntryKind::Other, true),
            Err(_) => (EntryKind::Other, !link),
        };
        entries.insert(key(&name, case_sensitive), Entry { name, kind, exists, link });
    }
    Some(entries)
}

/// Runs `job` on every index below `count` on up to 16 threads (no more than the processors), in no particular order.
pub(crate) fn parallel<T: Send>(count: usize, job: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get).clamp(1, 16);
    parallel_on(threads, count, job)
}

/// [`parallel`] for system calls: each waits on the file system and its filters (the virus scanner) more than it
/// computes, so more threads than processors finish sooner (32: on the real project's check, about 2 ms less than one
/// per processor on 12).
pub(crate) fn parallel_io<T: Send>(count: usize, job: impl Fn(usize) -> T + Sync) -> Vec<T> {
    parallel_on(32, count, job)
}

fn parallel_on<T: Send>(threads: usize, count: usize, job: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = threads.min(count.max(1));
    if threads <= 1 {
        return (0..count).map(&job).collect();
    }
    let next = AtomicUsize::new(0);
    let mut results: Vec<(usize, T)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= count {
                            return done;
                        }
                        done.push((i, job(i)));
                    }
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("a check thread")).collect()
    });
    results.sort_by_key(|(i, _)| *i);
    results.into_iter().map(|(_, r)| r).collect()
}

/// A path's directory key and name key, as a check looks the path up (`None`: a root).
type PathKeys = Option<(String, String)>;

/// A log's questions as a check asks them, with the keys it looks their answers up by: made on a log's first check
/// and kept while the log is not written again (a load's logs are not, after the load).
pub(crate) struct Prepared {
    case_sensitive: bool,
    version: u64,
    /// Each path asked about, with its directory's and its own key (`None`: a root).
    paths: Vec<(String, Seen, PathKeys)>,
    /// Each listing, with its directory's key.
    listings: Vec<(String, ListingView, u64, String)>,
    /// Each text read, with the stamp it was read under, its own key and its directory's and name's.
    texts: Vec<(String, u64, Option<FileStamp>, String, PathKeys)>,
    realpaths: Vec<(String, String)>,
    /// The directories the questions need listed, with their keys.
    dirs: Vec<(String, String)>,
}

impl Prepared {
    fn new(log: &ReadLog, case_sensitive: bool, version: u64) -> Prepared {
        let mut dirs: Vec<(String, String)> = Vec::new();
        let mut seen_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut want = |dir: &str, dir_key: &str, dirs: &mut Vec<(String, String)>| {
            if seen_dirs.insert(dir_key.to_owned()) {
                dirs.push((dir.to_owned(), dir_key.to_owned()));
            }
        };
        let split_keys = |path: &str| {
            split(path).map(|(dir, name)| (dir.to_owned(), key(dir, case_sensitive), key(name, case_sensitive)))
        };
        let mut paths = Vec::new();
        for (path, seen) in log.entries() {
            let keys = split_keys(&path).map(|(dir, dir_key, name_key)| {
                want(&dir, &dir_key, &mut dirs);
                (dir_key, name_key)
            });
            paths.push((path, seen, keys));
        }
        let mut listings = Vec::new();
        for (dir, view, hash) in log.listings() {
            let dir_key = key(&dir, case_sensitive);
            want(&dir, &dir_key, &mut dirs);
            listings.push((dir, view, hash, dir_key));
        }
        let mut texts = Vec::new();
        for (path, hash) in log.contents() {
            let stamp = log.content_stamp(&path);
            let own_key = key(&path, case_sensitive);
            let keys = split_keys(&path).map(|(dir, dir_key, name_key)| {
                want(&dir, &dir_key, &mut dirs);
                (dir_key, name_key)
            });
            texts.push((path, hash, stamp, own_key, keys));
        }
        Prepared { case_sensitive, version, paths, listings, texts, realpaths: log.realpaths(), dirs }
    }
}

impl ReadLog {
    /// This log's questions, prepared ([`Prepared`]).
    fn prepared(&self, case_sensitive: bool) -> Arc<Prepared> {
        let version = self.version();
        let mut cached = self.prepared.lock().expect("read log");
        if let Some(prepared) = cached.as_ref().filter(|p| p.version == version && p.case_sensitive == case_sensitive) {
            return Arc::clone(prepared);
        }
        let prepared = Arc::new(Prepared::new(self, case_sensitive, version));
        *cached = Some(Arc::clone(&prepared));
        prepared
    }
}

/// Directory listings shared by the logs one check asks ([`ReadLog::recheck_all`]): each directory listed once.
struct Views {
    index: HashMap<String, usize>,
    views: Vec<View>,
}

impl Views {
    fn view_of(&self, dir_key: &str) -> &View {
        &self.views[self.index[dir_key]]
    }

    /// The entry a path's keys ([`Prepared`]) name.
    fn entry_of(&self, keys: Option<&(String, String)>) -> Option<&Entry> {
        let (dir_key, name_key) = keys?;
        self.view_of(dir_key).as_ref()?.get(name_key).filter(|e| e.exists)
    }
}

/// One system call of a check: every log's questions are asked in one parallel pass ([`ReadLog::recheck_all`]).
enum Ask<'a> {
    /// List a directory.
    List(&'a str),
    /// Stamp a program file.
    Stat(&'a str),
    /// Read a text and compare it with its hash, unless its stamp is the one the read was made under.
    Read(&'a str, u64, Option<FileStamp>),
    /// Resolve a path's links.
    Realpath(&'a str),
}

/// An answer to an [`Ask`].
enum Answer {
    Listed(View),
    Stat(PathStat),
    SameText(bool),
    Realpath(String),
}

/// One log's questions in one check: the log's, and the program's files, whose texts their stamps answer for.
struct Questions<'a> {
    prepared: Arc<Prepared>,
    /// Indices of the texts to compare (not the program's files).
    texts: Vec<usize>,
    program: ProgramStamps<'a>,
}

impl ReadLog {
    /// Asks every question of this log again ([`crate::read_check`]). `program` are the files of the program the load
    /// made, with the stamps the source cache has for them.
    pub fn recheck(&self, program: ProgramStamps<'_>, case_sensitive: bool) -> Recheck {
        Self::recheck_all(&[(self, program)], case_sensitive).pop().expect("one answer per log")
    }

    /// [`ReadLog::recheck`] for several logs at once (a load's tsconfig parse, program and setup each keep one). Every
    /// question of every log is asked in one parallel pass, each directory any of them asks about listed once (the
    /// logs share most of them), and each log's answer comes from those answers.
    pub fn recheck_all(logs: &[(&ReadLog, ProgramStamps<'_>)], case_sensitive: bool) -> Vec<Recheck> {
        let questions: Vec<Questions<'_>> = logs
            .iter()
            .map(|&(log, program)| {
                let prepared = log.prepared(case_sensitive);
                let texts = if program.is_empty() {
                    (0..prepared.texts.len()).collect()
                } else {
                    let program_keys: std::collections::HashSet<String> =
                        program.iter().map(|(file, _)| key(file, case_sensitive)).collect();
                    (0..prepared.texts.len()).filter(|&i| !program_keys.contains(&prepared.texts[i].3)).collect()
                };
                Questions { prepared, texts, program }
            })
            .collect();

        // the directories to list, each once
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut asks: Vec<Ask<'_>> = Vec::new();
        for q in &questions {
            for (dir, dir_key) in &q.prepared.dirs {
                if let std::collections::hash_map::Entry::Vacant(entry) = index.entry(dir_key.clone()) {
                    entry.insert(asks.len());
                    asks.push(Ask::List(dir));
                }
            }
        }
        let listed = asks.len();
        for q in &questions {
            asks.extend(q.program.iter().map(|(file, _)| Ask::Stat(file)));
            asks.extend(q.texts.iter().map(|&i| {
                let (path, hash, stamp, _, _) = &q.prepared.texts[i];
                Ask::Read(path, *hash, *stamp)
            }));
            asks.extend(q.prepared.realpaths.iter().map(|(path, _)| Ask::Realpath(path)));
        }
        let mut answers = parallel_io(asks.len(), |i| match asks[i] {
            Ask::List(dir) => Answer::Listed(list(dir, case_sensitive)),
            Ask::Stat(file) => Answer::Stat(stamp::stat(file)),
            Ask::Read(path, hash, stamp) => Answer::SameText(
                stamp.is_some_and(|stamp| stamp::stat(path).file().is_some_and(|now| now.same(&stamp)))
                    || tsrs_vfs::osvfs::fs().read_file(path).is_some_and(|text| text_hash(&text) == hash),
            ),
            Ask::Realpath(path) => Answer::Realpath(tsrs_vfs::osvfs::fs().realpath(path)),
        })
        .into_iter();
        let views: Vec<View> = answers
            .by_ref()
            .take(listed)
            .map(|a| match a {
                Answer::Listed(view) => view,
                _ => unreachable!("a listing's answer"),
            })
            .collect();
        let views = Views { index, views };
        questions.iter().map(|q| q.answer(&views, &mut answers)).collect()
    }
}

impl Questions<'_> {
    /// The log's answer from the pass's answers (its own come next in `answers`, in the order they were asked).
    fn answer(&self, views: &Views, answers: &mut impl Iterator<Item = Answer>) -> Recheck {
        let prepared = &self.prepared;
        let stats: Vec<PathStat> = answers
            .by_ref()
            .take(self.program.len())
            .map(|a| match a {
                Answer::Stat(stat) => stat,
                _ => unreachable!("a stamp's answer"),
            })
            .collect();
        let same_texts: Vec<bool> = answers
            .by_ref()
            .take(self.texts.len())
            .map(|a| match a {
                Answer::SameText(same) => same,
                _ => unreachable!("a text's answer"),
            })
            .collect();
        let resolved: Vec<String> = answers
            .by_ref()
            .take(prepared.realpaths.len())
            .map(|a| match a {
                Answer::Realpath(path) => path,
                _ => unreachable!("a realpath's answer"),
            })
            .collect();

        // existence and kinds
        for (path, seen, keys) in &prepared.paths {
            if keys.is_none() {
                // a root: there, as a directory
                let is_dir = std::fs::metadata(path).is_ok_and(|m| m.is_dir());
                if seen.contradicted(false, is_dir) {
                    return Recheck::Different(format!("{path}: no longer as read"));
                }
                continue;
            }
            let entry = views.entry_of(keys.as_ref());
            let is_dir = entry.is_some_and(|e| e.kind == EntryKind::Dir);
            let is_file = entry.is_some_and(|e| e.kind != EntryKind::Dir);
            if seen.contradicted(is_file, is_dir) {
                return Recheck::Different(format!("{path}: no longer as read"));
            }
        }
        // listings, as their readers saw them
        for (dir, view, hash, dir_key) in &prepared.listings {
            let entries =
                views.view_of(dir_key).iter().flat_map(|v| v.values()).map(|e| (e.name.clone(), e.kind, e.link));
            if listing_hash(*view, entries) != *hash {
                return Recheck::Different(format!("{dir}: its entries changed"));
            }
        }
        // the program's files, each asked itself: the ones whose stamp changed, or was not settled, are parsed again
        let mut changed = Vec::new();
        for ((file, stamp), now) in self.program.iter().zip(stats) {
            let Some(now) = now.file() else {
                return Recheck::Different(format!("{file}: gone"));
            };
            let Some(stamp) = stamp else {
                return Recheck::Different(format!("{file}: not in the source cache"));
            };
            if !stamp.settled || !stamp.same(&now) {
                changed.push(file.clone());
            }
        }
        // texts read (but the program's files): the same text
        for (&i, same) in self.texts.iter().zip(same_texts) {
            let (path, _, _, _, keys) = &prepared.texts[i];
            if views.entry_of(keys.as_ref()).is_none_or(|e| e.kind == EntryKind::Dir) {
                return Recheck::Different(format!("{path}: gone"));
            }
            if !same {
                return Recheck::Different(format!("{path}: its text changed"));
            }
        }
        // links followed
        for ((path, answer), now) in prepared.realpaths.iter().zip(resolved) {
            if now != *answer {
                return Recheck::Different(format!("{path}: resolves elsewhere"));
            }
        }
        changed.sort();
        Recheck::Same { changed }
    }
}

#[cfg(test)]
mod tests {
    use super::{Recheck, split};
    use crate::read_log::{EntryKind, ListingView, ReadLog, Seen};
    use crate::stamp;

    #[test]
    fn splits() {
        assert_eq!(split("E:/a/b.ts"), Some(("E:/a", "b.ts")));
        assert_eq!(split("E:\\a\\b"), Some(("E:\\a", "b")));
        assert_eq!(split("E:/a/"), Some(("E:/", "a")));
        assert_eq!(split("E:/"), None);
        assert_eq!(split("/a"), Some(("/", "a")));
    }

    /// Each kind of read, read the same and then differently.
    #[test]
    fn rechecks_every_kind_of_read() {
        let dir = std::env::temp_dir().join(format!("rbxts-rs-read-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pkg")).unwrap();
        let root = dir.to_string_lossy().replace('\\', "/");
        let at = |p: &str| format!("{root}/{p}");
        std::fs::write(at("pkg/package.json"), "{}").unwrap();
        std::fs::write(at("a.ts"), "export {};").unwrap();
        let case_sensitive = !cfg!(windows);
        // settled stamps (as one taken two seconds after its file changed is)
        let stamp = |p: &str| stamp::stat(p).file().map(|s| stamp::FileStamp { settled: true, ..s });
        let program = vec![(at("a.ts"), stamp(&at("a.ts")))];
        let log = ReadLog::default();
        log.see_text(&at("pkg/package.json"), Some("{}"));
        log.see(&at("pkg/index.d.ts"), Seen::NO_FILE);
        log.see(&at("missing/deeper/x.ts"), Seen::NO_FILE | Seen::NO_DIR);
        log.see_listing(&at("pkg"), ListingView::Ts, [("package.json".to_owned(), EntryKind::File, false)]);
        let check = |program: &[super::ProgramStamp]| log.recheck(program, case_sensitive);
        assert_eq!(check(&program), Recheck::Same { changed: Vec::new() });
        assert_eq!(check(&program), Recheck::Same { changed: Vec::new() });
        // a stamp not settled: the file is parsed again (the source cache compares its text)
        let unsettled = vec![(at("a.ts"), stamp(&at("a.ts")).map(|s| stamp::FileStamp { settled: false, ..s }))];
        assert_eq!(check(&unsettled), Recheck::Same { changed: vec![at("a.ts")] });

        // a program file edited: to parse again
        std::fs::write(at("a.ts"), "export const a = 1;").unwrap();
        assert_eq!(check(&program), Recheck::Same { changed: vec![at("a.ts")] });
        // a probed path appears (and the listing changes with it)
        std::fs::write(at("pkg/index.d.ts"), "").unwrap();
        assert!(matches!(check(&program), Recheck::Different(_)));
        std::fs::remove_file(at("pkg/index.d.ts")).unwrap();
        assert!(matches!(check(&program), Recheck::Same { .. }));
        // a missing directory on the way to a probe appears: nothing below it yet, the same
        std::fs::create_dir_all(at("missing/deeper")).unwrap();
        assert!(matches!(check(&program), Recheck::Same { .. }));
        std::fs::write(at("missing/deeper/x.ts"), "").unwrap();
        assert!(matches!(check(&program), Recheck::Different(_)));
        std::fs::remove_dir_all(at("missing")).unwrap();
        // a JSON file rewritten with the same text, then with another
        std::fs::write(at("pkg/package.json"), "{}").unwrap();
        assert!(matches!(check(&program), Recheck::Same { .. }));
        std::fs::write(at("pkg/package.json"), "{ \"types\": \"x\" }").unwrap();
        assert!(matches!(check(&program), Recheck::Different(_)));
        // the same size, the old modification time set back, after a check found the file the same
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let set_old = |p: &str| std::fs::File::options().write(true).open(p).unwrap().set_modified(old).unwrap();
        std::fs::write(at("pkg/package.json"), "{}").unwrap();
        set_old(&at("pkg/package.json"));
        assert!(matches!(check(&program), Recheck::Same { .. }));
        std::fs::write(at("pkg/package.json"), "[]").unwrap();
        set_old(&at("pkg/package.json"));
        assert!(matches!(check(&program), Recheck::Different(_)));
        std::fs::write(at("pkg/package.json"), "{}").unwrap();
        // a program file rewritten with the same size and its modification time set back
        let program = vec![(at("a.ts"), stamp(&at("a.ts")))];
        assert_eq!(check(&program), Recheck::Same { changed: Vec::new() });
        let before = std::fs::metadata(at("a.ts")).unwrap().modified().unwrap();
        std::fs::write(at("a.ts"), "export const a = 2;").unwrap();
        std::fs::File::options().write(true).open(at("a.ts")).unwrap().set_modified(before).unwrap();
        assert_eq!(check(&program), Recheck::Same { changed: vec![at("a.ts")] });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
