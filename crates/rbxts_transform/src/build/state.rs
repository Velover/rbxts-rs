//! The compiler's incremental state on disk (DESIGN §14), for an `incremental` or `composite` project: what the
//! next build in a new process needs to transform only the changed closure.
//!
//! It sits beside roblox-ts's build info, as `<base>.rbxts-rs.buildinfo` (roblox-ts's `<base>.rbxtsc.tsbuildinfo`
//! has another format), and is written last in the commit (after the plugins' state files), so a crash never
//! leaves it newer than what it describes. It holds the compiler's identity, the plugins' names and versions, the
//! salt (options, configs, Rojo files), each program file's facts, and the artifacts the build wrote. A different
//! compiler binary or plugin version, or an unreadable file, makes the next build a clean one.
//!
//! Plugin file parts are not stored: `rbxts_plugin` has no way to serialise them yet. An incremental build with
//! plugins therefore transforms every file (with the plugins' states starting fresh, as the API says).

use std::path::Path;

use rustc_hash::FxHashMap;
use serde_json::{Map, Value, json};

use super::graph::{FileFacts, Version};

/// The state file's format version.
const FORMAT: u64 = 1;

/// The incremental state of the last committed build.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskState {
    /// The compiler binary's identity ([`compiler_identity`]).
    pub compiler: String,
    /// (name, version) of each active plugin, in registration order.
    pub plugins: Vec<(String, String)>,
    pub salt: Version,
    /// By path key.
    pub files: FxHashMap<String, FileFacts>,
    /// Plugin artifacts the build wrote (absolute paths).
    pub artifacts: Vec<String>,
}

/// The compiler's identity: the running executable's path, size and modification time, plus the roblox-ts version
/// it follows. Rebuilding the compiler changes it, so outputs of an older compiler are never kept.
pub fn compiler_identity() -> String {
    let exe = std::env::current_exe().ok();
    let meta = exe.as_ref().and_then(|e| std::fs::metadata(e).ok());
    let modified = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    format!(
        "rbxts-rs {} (roblox-ts {}) {} {} {modified}",
        env!("CARGO_PKG_VERSION"),
        crate::shared::constants::COMPILER_VERSION,
        exe.map(|e| e.to_string_lossy().into_owned()).unwrap_or_default(),
        meta.map_or(0, |m| m.len()),
    )
}

/// Where the state of a project with roblox-ts build info at `build_info` goes.
pub fn state_path(build_info: &str) -> String {
    let base = build_info.strip_suffix(".rbxtsc.tsbuildinfo").or_else(|| build_info.strip_suffix(".tsbuildinfo"));
    format!("{}.rbxts-rs.buildinfo", base.unwrap_or(build_info))
}

fn strings(value: &Value) -> Option<Vec<String>> {
    value.as_array()?.iter().map(|v| v.as_str().map(str::to_owned)).collect()
}

impl DiskState {
    /// Reads the state; `None` when it is missing, unreadable or of another format.
    pub fn read(path: &str) -> Option<DiskState> {
        let text = std::fs::read_to_string(path).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        if value.get("format")?.as_u64()? != FORMAT {
            return None;
        }
        let plugins = value
            .get("plugins")?
            .as_array()?
            .iter()
            .map(|p| Some((p.get(0)?.as_str()?.to_owned(), p.get(1)?.as_str()?.to_owned())))
            .collect::<Option<Vec<_>>>()?;
        let mut files = FxHashMap::default();
        for (key, file) in value.get("files")?.as_object()? {
            files.insert(
                key.clone(),
                FileFacts {
                    file_name: file.get("name")?.as_str()?.to_owned(),
                    version: Version::from_hex(file.get("v")?.as_str()?)?,
                    references: strings(file.get("refs")?)?,
                    augments: strings(file.get("aug")?)?,
                    global: file.get("global")?.as_bool()?,
                },
            );
        }
        Some(DiskState {
            compiler: value.get("compiler")?.as_str()?.to_owned(),
            plugins,
            salt: Version::from_hex(value.get("salt")?.as_str()?)?,
            files,
            artifacts: strings(value.get("artifacts")?)?,
        })
    }

    /// The state as the file's bytes (files sorted, so equal states give equal bytes).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut keys: Vec<&String> = self.files.keys().collect();
        keys.sort();
        let mut files = Map::new();
        for key in keys {
            let f = &self.files[key];
            files.insert(
                key.clone(),
                json!({
                    "name": f.file_name,
                    "v": f.version.to_hex(),
                    "refs": f.references,
                    "aug": f.augments,
                    "global": f.global,
                }),
            );
        }
        let value = json!({
            "format": FORMAT,
            "compiler": self.compiler,
            "plugins": self.plugins.iter().map(|(n, v)| json!([n, v])).collect::<Vec<_>>(),
            "salt": self.salt.to_hex(),
            "files": Value::Object(files),
            "artifacts": self.artifacts,
        });
        let mut bytes = serde_json::to_vec(&value).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }

    /// Writes the state (atomically, only if changed).
    pub fn write(&self, path: &str) -> std::io::Result<bool> {
        crate::plugins::lifecycle::write_if_changed(Path::new(path), &self.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_round_trips() {
        let mut files = FxHashMap::default();
        files.insert(
            "e:/p/src/a.ts".to_owned(),
            FileFacts {
                file_name: "E:/p/src/a.ts".into(),
                version: Version::of("x"),
                references: vec!["e:/p/src/b.ts".into(), "?missing".into()],
                augments: vec![],
                global: false,
            },
        );
        let state = DiskState {
            compiler: "c".into(),
            plugins: vec![("stamp".into(), "0.1.0".into())],
            salt: Version::of("salt"),
            files,
            artifacts: vec!["E:/p/out/x.json".into()],
        };
        let dir = std::env::temp_dir().join(format!("rbxts-rs-state-{}", std::process::id()));
        let path = dir.join("tsconfig.rbxts-rs.buildinfo");
        let path = path.to_string_lossy().into_owned();
        assert!(state.write(&path).unwrap());
        assert!(!state.write(&path).unwrap());
        assert_eq!(DiskState::read(&path), Some(state));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(state_path("E:/p/out/tsconfig.rbxtsc.tsbuildinfo"), "E:/p/out/tsconfig.rbxts-rs.buildinfo");
    }
}
