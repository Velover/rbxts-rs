//! Known divergences from a parity reference: the outputs that differ from stock roblox-ts's on purpose, because
//! rbxts-rs fixes a roblox-ts bug there (`parity/divergences.json`).
//!
//! Byte parity with a stock reference is the main regression check, and a fix makes some files differ from it. The
//! manifest records, per reference corpus, each such file with the fixes that explain it and the hash of the exact
//! output that was checked against them. `build --parity REF --divergences FILE --corpus NAME` then reports a
//! differing file whose output still has that hash as `known-fix` (identical but for known fixes), and anything else
//! as before; it also reports a listed file that is identical to the reference again as `stale` (a fix no longer
//! shows there). A known-fix file passes; a stale or differing one fails the run, so any further change to a listed
//! file is looked at again.
//!
//! The manifest stores hashes only (`rbxts_project::text_hash`, FNV-1a), not outputs, and, for a corpus with
//! `"hash_paths": true` (a private project), the hash of each file's path instead of the path.
//!
//! Recording (`--record-divergences FIX`, after checking every new difference by hand): each differing file that is
//! not listed is added with the fix `FIX`; a listed file whose output changed gets the new hash, and `FIX` if it does
//! not have it; stale entries are dropped. A file is never listed without a fix that names it, and the run still
//! prints every file it changed the manifest for.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

/// One listed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub fixes: Vec<String>,
    /// `text_hash` of the output, in hex.
    pub output: String,
}

/// The manifest, with one corpus selected.
pub struct Divergences {
    path: String,
    document: Value,
    corpus: String,
    hash_paths: bool,
    files: BTreeMap<String, Entry>,
}

/// A hash in the manifest's spelling.
pub fn hash(text: &str) -> String {
    format!("{:016x}", rbxts_project::text_hash(text))
}

impl Divergences {
    /// Reads `path` (a missing file is an empty manifest) and selects `corpus`.
    pub fn load(path: &str, corpus: &str) -> Result<Divergences, String> {
        let document = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
            Err(e) => return Err(format!("{path}: {e}")),
        };
        let section = &document["corpora"][corpus];
        let hash_paths = section["hash_paths"].as_bool().unwrap_or(false);
        let mut files = BTreeMap::new();
        if let Some(listed) = section["files"].as_object() {
            for (key, entry) in listed {
                let fixes = entry["fixes"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default();
                let output = entry["output"].as_str().unwrap_or_default().to_owned();
                files.insert(key.clone(), Entry { fixes, output });
            }
        }
        Ok(Divergences { path: path.to_owned(), document, corpus: corpus.to_owned(), hash_paths, files })
    }

    /// The key of a file: its path, or the path's hash.
    pub fn key(&self, relative: &str) -> String {
        if self.hash_paths { hash(relative) } else { relative.to_owned() }
    }

    /// The listed entry of a file.
    pub fn get(&self, relative: &str) -> Option<&Entry> {
        self.files.get(&self.key(relative))
    }

    /// The listed files.
    #[cfg(test)]
    pub fn listed(&self) -> impl Iterator<Item = (&String, &Entry)> {
        self.files.iter()
    }

    /// The description of a fix id, if the manifest has one.
    pub fn describe(&self, fix: &str) -> Option<&str> {
        self.document["fixes"][fix].as_str()
    }

    /// Records the run (see the module doc): `differing` holds each differing file and its output, `identical` the
    /// files identical to the reference. Returns one line per change, and writes the manifest.
    pub fn record(
        &mut self,
        fix: &str,
        differing: &[(String, String)],
        identical: &[String],
    ) -> Result<Vec<String>, String> {
        let mut changes = Vec::new();
        for (relative, output) in differing {
            let key = self.key(relative);
            let output = hash(output);
            match self.files.get_mut(&key) {
                Some(entry) if entry.output == output => {}
                Some(entry) => {
                    entry.output = output;
                    if !entry.fixes.iter().any(|f| f == fix) {
                        entry.fixes.push(fix.to_owned());
                    }
                    changes.push(format!("updated {relative} ({})", entry.fixes.join(", ")));
                }
                None => {
                    self.files.insert(key, Entry { fixes: vec![fix.to_owned()], output });
                    changes.push(format!("added   {relative} ({fix})"));
                }
            }
        }
        for relative in identical {
            if self.files.remove(&self.key(relative)).is_some() {
                changes.push(format!("dropped {relative} (identical to the reference again)"));
            }
        }
        self.write()?;
        Ok(changes)
    }

    fn write(&mut self) -> Result<(), String> {
        let mut files = Map::new();
        for (key, entry) in &self.files {
            let mut fixes = entry.fixes.clone();
            fixes.sort();
            let mut value = Map::new();
            value.insert("fixes".into(), Value::from(fixes));
            value.insert("output".into(), Value::from(entry.output.clone()));
            files.insert(key.clone(), Value::Object(value));
        }
        if !self.document.is_object() {
            self.document = Value::Object(Map::new());
        }
        let root = self.document.as_object_mut().expect("an object");
        let corpora = root.entry("corpora").or_insert_with(|| Value::Object(Map::new()));
        let mut sorted: BTreeMap<String, Value> =
            corpora.as_object().map(|m| m.clone().into_iter().collect()).unwrap_or_default();
        let mut section = Map::new();
        section.insert("hash_paths".into(), Value::from(self.hash_paths));
        section.insert("files".into(), Value::Object(files));
        sorted.insert(self.corpus.clone(), Value::Object(section));
        *corpora = Value::Object(sorted.into_iter().collect());
        let text = serde_json::to_string_pretty(&self.document).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, format!("{text}\n")).map_err(|e| format!("{}: {e}", self.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_checks_and_drops() {
        let path = std::env::temp_dir().join(format!("rbxts-bench-divergences-{}.json", std::process::id()));
        let path = path.to_string_lossy().into_owned();
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "{\"fixes\": {\"a\": \"fix a\"}, \"corpora\": {\"p\": {\"hash_paths\": true}}}").unwrap();

        let mut d = Divergences::load(&path, "p").unwrap();
        let changes = d.record("a", &[("x.luau".into(), "one".into())], &[]).unwrap();
        assert_eq!(changes, ["added   x.luau (a)"]);
        let d = Divergences::load(&path, "p").unwrap();
        assert_eq!(d.get("x.luau"), Some(&Entry { fixes: vec!["a".into()], output: hash("one") }));
        assert_eq!(d.describe("a"), Some("fix a"));
        // the path is stored hashed
        assert!(!std::fs::read_to_string(&path).unwrap().contains("x.luau"));

        let mut d = Divergences::load(&path, "p").unwrap();
        let changes = d.record("b", &[("x.luau".into(), "two".into())], &[]).unwrap();
        assert_eq!(changes, ["updated x.luau (a, b)"]);
        let mut d = Divergences::load(&path, "p").unwrap();
        let changes = d.record("b", &[], &["x.luau".into()]).unwrap();
        assert_eq!(changes, ["dropped x.luau (identical to the reference again)"]);
        assert_eq!(Divergences::load(&path, "p").unwrap().listed().count(), 0);
        std::fs::remove_file(&path).unwrap();
    }
}
