//! Declaration ids (the TS transformer's `util/uid.ts`): internal ids from paths, and the public id in each
//! `idGenerationMode`.
//!
//! Shaped for DESIGN §10.4. `full` ids are path-derived and formatted at once. `short`, `tiny` and `obfuscated` ids
//! are Hashids of a sequential counter, so they depend on allocation order: during the transform [`Ids::request`]
//! answers them with a late key (the internal id) and the file part records the new internal id; at `finish`,
//! [`Ids::assign`] walks the file parts in program order and allocates the counters, exactly as one checker
//! transforming the files in that order would. Every new id, `full` ones too, enters the identifier table in that
//! order, so `flamework.build` is the same for any checker count.

use std::collections::HashMap;

use crate::build_info::BuildInfo;
use crate::hashids::Hashids;
use crate::{js, node_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdMode {
    Full,
    Short,
    Tiny,
    Obfuscated,
}

impl IdMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "full" => Some(IdMode::Full),
            "short" => Some(IdMode::Short),
            "tiny" => Some(IdMode::Tiny),
            "obfuscated" => Some(IdMode::Obfuscated),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            IdMode::Full => "full",
            IdMode::Short => "short",
            IdMode::Tiny => "tiny",
            IdMode::Obfuscated => "obfuscated",
        }
    }
}

/// `formatInternalid`: drops the package name and the first directory of the path (`out`) from an internal id and
/// prefixes `hash_prefix` when it is set and not empty. An id that does not match `^.*:(.*)@(.+)$` is returned as it
/// is.
pub fn format_internal_id(internal_id: &str, hash_prefix: Option<&str>) -> String {
    let Some((path, name)) = split_internal_id(internal_id) else {
        return internal_id.to_owned();
    };
    // `path.replace(/^(.*?)[\/\\]/, "")`, where `.` stops at a line terminator.
    let revised = match path.find(['/', '\\']) {
        Some(i) if !path[..i].chars().any(js::is_line_terminator) => &path[i + 1..],
        _ => path,
    };
    match hash_prefix.filter(|p| !p.is_empty()) {
        Some(prefix) => format!("{prefix}:{revised}@{name}"),
        None => format!("{revised}@{name}"),
    }
}

/// `/^.*:(.*)@(.+)$/` (`.` does not match line terminators): the path and the name.
fn split_internal_id(id: &str) -> Option<(&str, &str)> {
    if id.chars().any(js::is_line_terminator) {
        return None;
    }
    // The greedy `.*:` takes the last colon that still leaves `(.*)@(.+)` matching after it, and the greedy `(.*)@`
    // takes the last `@` that leaves at least one character.
    for (colon, _) in id.match_indices(':').collect::<Vec<_>>().into_iter().rev() {
        let rest = &id[colon + 1..];
        let at = rest
            .char_indices()
            .filter(|(i, c)| *c == '@' && *i + 1 < rest.len())
            .map(|(i, _)| i)
            .next_back();
        if let Some(at) = at {
            return Some((&rest[..at], &rest[at + 1..]));
        }
    }
    None
}

/// The internal id of a declaration in one of the project's own files (`getInternalId`, first branch):
/// `<package>:<output path relative to the project, without .lua>@<declaration name>`.
pub fn project_internal_id(
    package_name: &str,
    current_directory: &str,
    output_path: &str,
    declaration_name: &str,
) -> String {
    let output = output_path.strip_suffix(".lua").unwrap_or(output_path);
    let relative = node_path::relative(current_directory, output);
    format!(
        "{package_name}:{}@{declaration_name}",
        relative.replace('\\', "/")
    )
}

/// The internal id of a declaration in a package's file (`getInternalId`, second branch):
/// `<package>:<file relative to the package, without (.d).ts, index as init>@<declaration name>`. As in the TS, the
/// `.` before `ts` is any character and `index` is replaced at the end of any name (`myindex` becomes `myinit`).
pub fn package_internal_id(
    package_name: &str,
    package_directory: &str,
    file_path: &str,
    declaration_name: &str,
) -> String {
    let stripped = strip_ts_extension(file_path);
    let stripped = match stripped.strip_suffix("index") {
        Some(base) => format!("{base}init"),
        None => stripped.to_owned(),
    };
    let relative = node_path::relative(package_directory, &stripped);
    format!(
        "{package_name}:{}@{declaration_name}",
        relative.replace('\\', "/")
    )
}

/// `.replace(/(\.d)?.ts$/, "")`.
fn strip_ts_extension(path: &str) -> &str {
    let Some(base) = path.strip_suffix("ts") else {
        return path;
    };
    let mut chars = base.chars();
    let Some(any) = chars.next_back() else {
        return path;
    };
    if js::is_line_terminator(any) {
        return path;
    }
    let base = chars.as_str();
    base.strip_suffix(".d").unwrap_or(base)
}

/// Whether a file is one of the project's own (`isPathDescendantOfAny(file, rootDirs)`).
pub fn is_project_file(file_path: &str, root_dirs: &[&str]) -> bool {
    root_dirs
        .iter()
        .any(|dir| node_path::is_path_descendant_of(file_path, dir))
}

/// `getDeclarationName`: the name with every named ancestor's before it, joined by dots (`Enum.Material.Air`),
/// from the outermost. `$p:error` when the declaration's own name is not an identifier.
pub fn declaration_name<'a>(
    own_name: Option<&'a str>,
    ancestors_outermost_first: impl IntoIterator<Item = &'a str>,
) -> String {
    let Some(own) = own_name else {
        return "$p:error".into();
    };
    let mut parts: Vec<&str> = ancestors_outermost_first.into_iter().collect();
    parts.push(own);
    parts.join(".")
}

/// The id settings of a compilation.
#[derive(Debug, Clone)]
pub struct IdConfig {
    pub mode: IdMode,
    /// `transformer.hashPrefix` (a package defaults it to its name before this is built).
    pub hash_prefix: Option<String>,
    pub package_name: String,
    pub is_game: bool,
    /// The salt: `transformer.salt`, else the build info's.
    pub salt: String,
}

impl IdConfig {
    /// `TransformState.hash(id, noPrefix)`.
    pub fn hash(&self, id: u64, no_prefix: bool) -> String {
        let hash = Hashids::new(&self.salt, 2).encode_one(id);
        let prefix = self.hash_prefix.as_deref();
        if (self.is_game && prefix.is_none_or(str::is_empty)) || no_prefix {
            hash
        } else {
            format!("{}:{hash}", prefix.unwrap_or(&self.package_name))
        }
    }
}

/// A declaration an id is asked for, as the compiler describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// From [`project_internal_id`] or [`package_internal_id`].
    pub internal_id: String,
    /// Whether the declaration is in a package (not one of the project's own files).
    pub is_package: bool,
    /// [`declaration_name`].
    pub name: String,
    /// The source file, for the package build info that covers it (`getBuildInfoFromFile`).
    pub source_file: String,
    /// The source file's name without its last extension (`path.parse(fileName).name`): `short` ids carry it, with
    /// `index` as `init`.
    pub file_name: String,
}

/// A new id the file part records, for [`Ids::assign`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewId {
    pub internal_id: String,
    pub name: String,
    pub file_name: String,
}

/// The answer to an id request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdAnswer {
    /// The id, known now.
    Now(String),
    /// Known at `finish`: emit `late_string(key)`; the key is the internal id.
    Late(String),
}

/// The id table during a compilation: read-only while files are transformed (any checker), filled at `finish`.
#[derive(Debug, Clone)]
pub struct Ids {
    pub config: IdConfig,
}

impl Ids {
    pub fn new(config: IdConfig) -> Self {
        Ids { config }
    }

    /// The id of a declaration (`getDeclarationUid`), reading the build info and its package build infos. A new id
    /// returns the record to push onto the file part (first request in the file only: the caller dedups by
    /// internal id per file).
    pub fn request(
        &self,
        build_info: &BuildInfo,
        declaration: &Declaration,
    ) -> (IdAnswer, Option<NewId>) {
        if let Some(id) = build_info.identifier_from_internal(&declaration.internal_id) {
            return (IdAnswer::Now(id), None);
        }
        if declaration.is_package {
            // The package did not generate an id: its internal id, formatted with its own prefix, keeps packages and
            // games from breaking each other.
            let file_prefix = build_info
                .build_info_from_file(&declaration.source_file)
                .and_then(BuildInfo::identifier_prefix);
            let id = match file_prefix {
                Some(prefix) => format_internal_id(&declaration.internal_id, Some(prefix)),
                None => declaration.internal_id.clone(),
            };
            return (IdAnswer::Now(id), None);
        }
        let record = NewId {
            internal_id: declaration.internal_id.clone(),
            name: declaration.name.clone(),
            file_name: declaration.file_name.clone(),
        };
        match self.config.mode {
            IdMode::Full => (
                IdAnswer::Now(format_internal_id(
                    &declaration.internal_id,
                    self.config.hash_prefix.as_deref(),
                )),
                Some(record),
            ),
            _ => (
                IdAnswer::Late(declaration.internal_id.clone()),
                Some(record),
            ),
        }
    }

    /// Allocates the new ids of every file part, in program order, into the build info's table. Returns the late
    /// strings to resolve (internal id → id); `full` ids are recorded but need no resolving.
    pub fn assign<'a>(
        &self,
        build_info: &mut BuildInfo,
        parts: impl IntoIterator<Item = &'a [NewId]>,
    ) -> HashMap<String, String> {
        let mut resolved = HashMap::new();
        for part in parts {
            for new in part {
                if let Some(id) = build_info.identifier_from_internal(&new.internal_id) {
                    resolved.insert(new.internal_id.clone(), id);
                    continue;
                }
                let id = self.make_id(build_info.latest_id(), new);
                build_info
                    .add_identifier(&new.internal_id, &id)
                    .expect("checked above");
                resolved.insert(new.internal_id.clone(), id);
            }
        }
        resolved
    }

    /// The id a new declaration gets with `counter` as the next sequential id.
    pub fn make_id(&self, counter: u64, new: &NewId) -> String {
        let prefix = self.config.hash_prefix.as_deref().filter(|p| !p.is_empty());
        match self.config.mode {
            IdMode::Full => format_internal_id(&new.internal_id, prefix),
            IdMode::Obfuscated => self.config.hash(counter, false),
            IdMode::Short | IdMode::Tiny => {
                let hash = self.config.hash(counter, true);
                let lua_file_name = if new.file_name == "index" {
                    "init"
                } else {
                    &new.file_name
                };
                let short = if self.config.mode == IdMode::Short {
                    format!("{lua_file_name}@{}{{{hash}}}", new.name)
                } else {
                    format!("{}{{{hash}}}", new.name)
                };
                match prefix {
                    Some(prefix) => format!("{prefix}:{short}"),
                    None => short,
                }
            }
        }
    }
}

/// `getCallsiteUuid`: the uuid v5 of `<package>:<fileId>@<declaration>+<offset>`, where `file_id` is the file
/// relative to the package root with forward slashes, `declaration` the enclosing named declaration (empty when
/// none) and `utf16_offset` the call's distance from that declaration's start in UTF-16 units (both as TS
/// `getStart()`), or from the file's start without one. The namespace is the constant one, or the build seed under
/// obfuscation, so that remote names rotate with every clean build.
pub fn callsite_uuid(
    package_name: &str,
    file_id: &str,
    declaration_name: &str,
    utf16_offset: u32,
    obfuscation_seed: Option<&crate::uuid::Namespace>,
) -> String {
    let key = format!("{package_name}:{file_id}@{declaration_name}+{utf16_offset}");
    match obfuscation_seed {
        Some(seed) => seed.v5(&key),
        None => crate::uuid::Namespace::callsite().v5(&key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callsite_uuids_are_v5_of_the_key() {
        // The uuid table has this key in the callsite namespace.
        assert_eq!(
            callsite_uuid(
                "transformer-fixture",
                "src/networking.ts",
                "Remotes",
                0,
                None
            ),
            crate::uuid::v5(
                "transformer-fixture:src/networking.ts@Remotes+0",
                crate::uuid::CALLSITE_NAMESPACE
            )
            .unwrap()
        );
    }
}
