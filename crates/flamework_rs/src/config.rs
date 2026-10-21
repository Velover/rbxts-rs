//! `flamework.config.json` (the TS transformer's `util/projectConfig.ts`): where it is, how it is read, the tsconfig
//! entry check, the runtime sections, and the `$schema` line. Error texts are the TS transformer's.


use crate::env::{self, Env};
use crate::inputs::{FsInputs, Inputs};
use crate::ids::IdMode;
use crate::json::{self, JsObject, JsValue};
use crate::packages::TRANSFORMER_PACKAGE;
use crate::{ProjectError, js, jsonc, node_path, schema};

/// The file every Flamework package reads its options from, found from the tsconfig's directory up to the package
/// root.
pub const PROJECT_CONFIG_NAME: &str = "flamework.config.json";

/// The schema of that file, shipped at the root of the transformer package.
pub const PROJECT_CONFIG_SCHEMA_NAME: &str = "flamework.config.schema.json";

/// Keys of the tsconfig entry that belong to the plugin loader rather than to Flamework, so they are never refused.
pub const LOADER_KEYS: [&str; 10] = [
    "transform",
    "import",
    "type",
    "after",
    "afterDeclarations",
    "name",
    "transformProgram",
    "isEsm",
    "tsConfig",
    "resolvePathAliases",
];

/// The sections the runtime packages read, compiled into `include/flamework/config.json` for a game.
pub const RUNTIME_SECTIONS: [&str; 5] = ["core", "networking", "components", "scopes", "testing"];

/// The keys of `testing` that `flamework-test` alone read before 2.0.0-beta.0, now in `flameworkTest`.
const MOVED_TESTING_KEYS: [&str; 6] = [
    "failOnSkip",
    "keepAwake",
    "lockTimeout",
    "lockHold",
    "parallel",
    "showWindows",
];

/// The keys of the top-level `cloud` section before 2.0.0-beta.0, and their names in `flameworkTest`.
const MOVED_CLOUD_KEYS: [(&str, &str); 4] = [
    ("testingUniverseId", "universeId"),
    ("testingPlaceId", "placeId"),
    ("apiKey", "apiKey"),
    ("originalPlace", "originalPlace"),
];

/// Where each `flamework-test` setting a config file still keeps in its old place now goes, one line each.
pub fn describe_moved_keys(config: &JsValue) -> Vec<String> {
    let Some(config) = config.as_object() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if let Some(JsValue::Object(testing)) = config.get("testing") {
        for key in MOVED_TESTING_KEYS {
            if testing.contains_key(key) {
                lines.push(format!("move 'testing.{key}' to 'flameworkTest.{key}'"));
            }
        }
    }
    if let Some(cloud) = config
        .get("cloud")
        .filter(|v| !matches!(v, JsValue::Undefined))
    {
        if let JsValue::Object(cloud) = cloud {
            for key in cloud.keys() {
                if let Some((_, moved)) = MOVED_CLOUD_KEYS.iter().find(|(old, _)| old == key) {
                    lines.push(format!("move 'cloud.{key}' to 'flameworkTest.{moved}'"));
                }
            }
        }
        lines.push("remove 'cloud': its keys are in 'flameworkTest' now".to_owned());
    }
    lines
}

/// Locates `flamework.config.json`: `explicit_path` (relative to the tsconfig's directory) when given, which must
/// exist; else the tsconfig's directory and each parent up to and including the package root.
pub fn find_project_config(
    project_directory: &str,
    root_directory: &str,
    explicit_path: Option<&str>,
) -> Result<Option<String>, ProjectError> {
    find_project_config_with(&mut FsInputs, project_directory, root_directory, explicit_path)
}

/// [`find_project_config`], reading through `inputs`.
pub fn find_project_config_with(
    inputs: &mut dyn Inputs,
    project_directory: &str,
    root_directory: &str,
    explicit_path: Option<&str>,
) -> Result<Option<String>, ProjectError> {
    if let Some(explicit) = explicit_path {
        let resolved = node_path::resolve(&[project_directory, explicit]);
        if !inputs.exists(&resolved) {
            return Err(ProjectError(format!(
                "The Flamework config file '{explicit}' does not exist (looked at '{resolved}')."
            )));
        }
        return Ok(Some(resolved));
    }
    let root = node_path::resolve(&[root_directory]);
    let mut current = node_path::resolve(&[project_directory]);
    loop {
        let candidate = node_path::join(&[&current, PROJECT_CONFIG_NAME]);
        if inputs.exists(&candidate) {
            return Ok(Some(candidate));
        }
        if node_path::relative(&current, &root).is_empty() {
            return Ok(None);
        }
        let parent = node_path::dirname(&current);
        if parent == current {
            return Ok(None);
        }
        current = parent;
    }
}

/// Reads and validates a `flamework.config.json`: comments and trailing commas allowed, `${NAME}` /
/// `${NAME:-fallback}` / `$$` substituted from `env`, strings converted where the schema expects a boolean, a
/// number or a list, then validated. The `$schema` key is dropped.
pub fn read_project_config(config_path: &str, env: &Env) -> Result<JsValue, ProjectError> {
    let text = std::fs::read_to_string(config_path)
        .map_err(|e| ProjectError(format!("Failed to read {config_path}: {e}")))?;
    parse_project_config(config_path, &text, env)
}

/// [`read_project_config`] on a text already read.
pub fn parse_project_config(
    config_path: &str,
    text: &str,
    env: &Env,
) -> Result<JsValue, ProjectError> {
    let text = text.to_owned();
    let parsed = jsonc::parse_config_text(&text)
        .map_err(|e| ProjectError(format!("Failed to parse {config_path}: {e}")))?;

    // Before the environment, so that an old key's own reference to a variable is not what fails.
    let moved = describe_moved_keys(&parsed);
    if !moved.is_empty() {
        return Err(ProjectError(format!(
            "Invalid {config_path}: flamework-test's settings have a section of their own, \"flameworkTest\":\n  {}",
            moved.join("\n  ")
        )));
    }

    let describe = |pointer: &str| {
        format!(
            "{config_path}: '{}'",
            if pointer.is_empty() { "/" } else { pointer }
        )
    };
    let (substituted, _) = env::substitute_env(&parsed, env, &describe)?;
    let schema = schema::project_config_schema();
    let config = env::coerce_by_schema(&substituted, schema, &describe)?;

    if let Err(errors) = schema::validate(schema, schema, &config) {
        let lines: Vec<String> = errors.iter().map(schema::SchemaError::describe).collect();
        return Err(ProjectError(format!(
            "Invalid {config_path}:\n  {}",
            lines.join("\n  ")
        )));
    }

    let mut project = config;
    if let Some(object) = project.as_object_mut() {
        object.remove("$schema");
    }
    Ok(project)
}

/// The transformer options a tsconfig entry may not set: the `transformer` section's keys.
fn transformer_option_keys() -> Vec<&'static str> {
    schema::project_config_schema()
        .get("properties")
        .and_then(|p| p.get("transformer"))
        .and_then(|t| t.get("properties"))
        .and_then(JsValue::as_object)
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default()
}

/// Refuses a tsconfig entry that sets anything but the plugin loader's keys and `configFile`: transformer options are
/// read from the config file only. `config_path` is the file the options belong in, and `exists` says whether it is
/// there yet.
pub fn assert_transformer_entry(
    entry: &JsObject,
    config_path: &str,
    exists: bool,
) -> Result<(), ProjectError> {
    let option_keys = transformer_option_keys();
    let mut options = Vec::new();
    let mut unknown = Vec::new();
    for (key, value) in entry.iter() {
        if matches!(value, JsValue::Undefined)
            || key == "configFile"
            || LOADER_KEYS.contains(&key.as_str())
        {
            continue;
        }
        if option_keys.contains(&key.as_str()) {
            &mut options
        } else {
            &mut unknown
        }
        .push(format!("'{key}'"));
    }
    if options.is_empty() && unknown.is_empty() {
        return Ok(());
    }
    let mut lines = vec![format!(
        "The tsconfig entry for {TRANSFORMER_PACKAGE} takes only \"transform\" and \"configFile\"; transformer options are read from {PROJECT_CONFIG_NAME}."
    )];
    if !options.is_empty() {
        let target = if exists {
            config_path.to_owned()
        } else {
            format!("{config_path}, a new file")
        };
        lines.push(format!(
            "Move {} to the \"transformer\" section of {target}.",
            options.join(", ")
        ));
    }
    if !unknown.is_empty() {
        lines.push(format!(
            "Remove {}: not a transformer option.",
            unknown.join(", ")
        ));
    }
    Err(ProjectError(lines.join("\n")))
}

/// Finds the config file for a tsconfig entry, after checking the entry.
pub fn locate_project_config(
    project_directory: &str,
    root_directory: &str,
    entry: &JsObject,
) -> Result<Option<String>, ProjectError> {
    locate_project_config_with(&mut FsInputs, project_directory, root_directory, entry)
}

/// [`locate_project_config`], reading through `inputs`.
pub fn locate_project_config_with(
    inputs: &mut dyn Inputs,
    project_directory: &str,
    root_directory: &str,
    entry: &JsObject,
) -> Result<Option<String>, ProjectError> {
    let config_file = match entry.get("configFile") {
        None | Some(JsValue::Undefined) => None,
        Some(JsValue::String(s)) => Some(s.as_str()),
        Some(_) => {
            return Err(ProjectError(format!(
                "\"configFile\" on the tsconfig entry for {TRANSFORMER_PACKAGE} must be a path, relative to the tsconfig's directory."
            )));
        }
    };
    let config_path =
        find_project_config_with(inputs, project_directory, root_directory, config_file)?;
    let target = match &config_path {
        Some(p) => p.clone(),
        None => node_path::join(&[project_directory, PROJECT_CONFIG_NAME]),
    };
    assert_transformer_entry(entry, &target, config_path.is_some())?;
    Ok(config_path)
}

/// A compilation's config, as `loadProjectConfig` returns it.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedProjectConfig {
    /// The `transformer` section (an empty object when there is none).
    pub config: JsValue,
    /// The whole file, or an empty object when there is none.
    pub project: JsValue,
    pub config_path: Option<String>,
    /// `.env` and `.env.local` next to the file (or next to the tsconfig when there is no file), with the process
    /// environment on top. `Flamework.env` reads it.
    pub env: Env,
}

impl LoadedProjectConfig {
    /// The typed transformer options.
    pub fn transformer(&self) -> TransformerConfig {
        TransformerConfig::from_value(&self.config)
    }
}

/// Resolves the project config, the transformer options and the environment for a compilation.
pub fn load_project_config(
    project_directory: &str,
    root_directory: &str,
    entry: &JsObject,
    process_env: &Env,
) -> Result<LoadedProjectConfig, ProjectError> {
    load_project_config_with(
        &mut crate::inputs::FsWithEnv(process_env),
        project_directory,
        root_directory,
        entry,
    )
}

/// [`load_project_config`], reading files and the process environment through `inputs`, which is asked only for
/// the variables that matter: those the `.env` files set and those the config file names (`${NAME` anywhere in
/// its text).
pub fn load_project_config_with(
    inputs: &mut dyn Inputs,
    project_directory: &str,
    root_directory: &str,
    entry: &JsObject,
) -> Result<LoadedProjectConfig, ProjectError> {
    let config_path =
        locate_project_config_with(inputs, project_directory, root_directory, entry)?;
    let env_directory = match &config_path {
        Some(p) => node_path::dirname(p),
        None => project_directory.to_owned(),
    };
    let config_text = match &config_path {
        Some(path) => Some(inputs.read(path).ok_or_else(|| {
            ProjectError(format!("Failed to read {path}: it could not be read"))
        })?),
        None => None,
    };
    let mut names = env::referenced_names(config_text.as_deref().unwrap_or(""));
    let env = env::load_env_with(inputs, &env_directory, &mut names);
    let Some(config_path) = config_path else {
        return Ok(LoadedProjectConfig {
            config: JsValue::Object(JsObject::new()),
            project: JsValue::Object(JsObject::new()),
            config_path: None,
            env,
        });
    };
    let project = parse_project_config(&config_path, config_text.as_deref().unwrap_or(""), &env)?;
    let config = match project.get("transformer") {
        Some(JsValue::Object(t)) => JsValue::Object(t.clone()),
        _ => JsValue::Object(JsObject::new()),
    };
    Ok(LoadedProjectConfig {
        config,
        project,
        config_path: Some(config_path),
        env,
    })
}

/// `fingerprintEmitConfig`: a hash of the options in the config file that shape the code the transformer emits
/// into each file: the whole `transformer` section, the `serialization` section and `networking.serialization`.
/// `flamework.build` records it as `inputs.config`; a build that reuses that file refuses another
/// (`TransformState.refuseChangedInputs`; rbxts-rs: TODO(S2), incremental builds).
pub fn fingerprint_emit_config(loaded: &LoadedProjectConfig) -> String {
    let mut snapshot = JsObject::new();
    snapshot.insert("transformer", loaded.config.clone());
    snapshot.insert(
        "serialization",
        loaded
            .project
            .get("serialization")
            .cloned()
            .unwrap_or(JsValue::Undefined),
    );
    snapshot.insert(
        "networkingSerialization",
        loaded
            .project
            .get("networking")
            .and_then(|n| n.get("serialization"))
            .cloned()
            .unwrap_or(JsValue::Undefined),
    );
    hash_stable(&JsValue::Object(snapshot))
}

/// `hashStable`: the sha1 (hex) of `JSON.stringify` of the value with every object's keys sorted.
pub fn hash_stable(value: &JsValue) -> String {
    fn stable(value: &JsValue) -> JsValue {
        match value {
            JsValue::Array(items) => JsValue::Array(items.iter().map(stable).collect()),
            JsValue::Object(object) => {
                let mut keys: Vec<&String> = object.keys().collect();
                // `Object.keys(value).sort()`: by UTF-16 code units. Integer-like keys come first in a
                // JavaScript object whatever the insertion order, and `JsObject` keeps that rule.
                keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
                let mut out = JsObject::new();
                for key in keys {
                    out.insert(key.clone(), stable(object.get(key).expect("a listed key")));
                }
                JsValue::Object(out)
            }
            other => other.clone(),
        }
    }
    let text = json::stringify(&stable(value), None).unwrap_or_default();
    crate::sha::hex(&crate::sha::sha1(text.as_bytes()))
}

/// The runtime sections of a project config, or `None` when it has none (no `config.json` artifact then).
pub fn get_runtime_config(project: &JsValue) -> Option<JsValue> {
    let mut runtime = JsObject::new();
    for section in RUNTIME_SECTIONS {
        if let Some(value) = project
            .get(section)
            .filter(|v| !matches!(v, JsValue::Undefined))
        {
            runtime.insert(section, value.clone());
        }
    }
    (!runtime.is_empty()).then_some(JsValue::Object(runtime))
}

/// One `transformer.plugins` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginEntry {
    pub path: String,
    pub options: Option<JsValue>,
}

/// The `transformer` section, typed. The file has been validated, so every field has its schema type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransformerConfig {
    pub plugins: Vec<PluginEntry>,
    pub no_semantic_diagnostics: Option<bool>,
    pub salt: Option<String>,
    pub hash_prefix: Option<String>,
    pub obfuscation: Option<bool>,
    pub id_generation_mode: Option<IdMode>,
    pub guard_generation_dedup_limit: Option<u32>,
}

impl TransformerConfig {
    pub fn from_value(value: &JsValue) -> Self {
        let string = |key: &str| value.get(key).and_then(JsValue::as_str).map(str::to_owned);
        let boolean = |key: &str| value.get(key).and_then(JsValue::as_bool);
        let plugins = value
            .get("plugins")
            .and_then(JsValue::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| match item {
                        JsValue::String(path) => Some(PluginEntry {
                            path: path.clone(),
                            options: None,
                        }),
                        JsValue::Object(o) => Some(PluginEntry {
                            path: o.get("path").and_then(JsValue::as_str)?.to_owned(),
                            options: o.get("options").cloned(),
                        }),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        TransformerConfig {
            plugins,
            no_semantic_diagnostics: boolean("noSemanticDiagnostics"),
            salt: string("salt"),
            hash_prefix: string("hashPrefix"),
            obfuscation: boolean("obfuscation"),
            id_generation_mode: string("idGenerationMode").and_then(|m| IdMode::parse(&m)),
            guard_generation_dedup_limit: value
                .get("optimizations")
                .and_then(|o| o.get("guardGenerationDedupLimit"))
                .and_then(JsValue::as_f64)
                .map(|n| n as u32),
        }
    }

    /// `idGenerationMode`, defaulting to `obfuscated` with obfuscation on, else `full`.
    pub fn effective_id_mode(&self) -> IdMode {
        self.id_generation_mode
            .unwrap_or(if self.obfuscation == Some(true) {
                IdMode::Obfuscated
            } else {
                IdMode::Full
            })
    }
}

// ------------------------------------------------------------------------------------------------ $schema line

/// The `$schema` value that points a config file at a schema file: relative, with forward slashes, starting with
/// `./` or `../`; a schema on another drive gets a `file://` URL.
pub fn get_schema_reference(config_path: &str, schema_path: &str) -> String {
    let relative = node_path::relative(&node_path::dirname(config_path), schema_path);
    if node_path::is_absolute(&relative) {
        return node_path::path_to_file_url(schema_path);
    }
    let reference = relative.replace('\\', "/");
    if reference.starts_with("../") {
        reference
    } else {
        format!("./{reference}")
    }
}

/// Adds `"$schema": reference` as the first key of a config file's text, keeping every other byte. `None` when the
/// file already has a `$schema` or is not an object that parses.
pub fn insert_schema_reference(text: &str, reference: &str) -> Option<String> {
    let parse = |source: &str| -> Option<(usize, Option<usize>, usize, bool)> {
        let file = jsonc::parse_json_text(source);
        if file.parse_error.is_some() {
            return None;
        }
        let root = file.root?;
        let jsonc::Kind::Object(members) = &root.kind else {
            return None;
        };
        let has_schema = members.iter().any(
            |m| matches!(&m.name, Some(jsonc::Name::String { value, .. }) if value == "$schema"),
        );
        Some((
            root.start,
            members.first().map(|m| m.start),
            root.end,
            has_schema,
        ))
    };

    let (open, first_member, end, has_schema) = parse(text)?;
    if has_schema {
        return None;
    }
    let mut property = String::from("\"$schema\": ");
    json::write_string(&mut property, reference);
    let first_token = first_member.unwrap_or(end - 1);
    let between = &text[open..first_token];
    let line_break = between.find('\n').map(|j| {
        if j > 0 && between.as_bytes()[j - 1] == b'\r' {
            j - 1
        } else {
            j
        }
    });

    let result = match line_break {
        None => {
            let separator = if first_member.is_some() {
                ","
            } else if text.as_bytes().get(open + 1) == Some(&b'}') {
                " "
            } else {
                ""
            };
            format!(
                "{} {property}{separator}{}",
                &text[..open + 1],
                &text[open + 1..]
            )
        }
        Some(line_break) => {
            let break_at = open + line_break;
            let newline = if text.as_bytes()[break_at] == b'\r' {
                "\r\n"
            } else {
                "\n"
            };
            let mut line_start = break_at + newline.len();
            while let Some(rel) = text[line_start..].find('\n') {
                let end = line_start + rel;
                if !js::trim(&text[line_start..end]).is_empty() {
                    break;
                }
                line_start = end + 1;
            }
            let mut indent: String = text[line_start..]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            if first_member.is_none() {
                indent.push('\t');
            }
            let comma = if first_member.is_some() { "," } else { "" };
            format!(
                "{}{indent}{property}{comma}{newline}{}",
                &text[..line_start],
                &text[line_start..]
            )
        }
    };

    // Never write a file the loader could no longer read.
    match parse(&result) {
        Some((_, _, _, true)) => Some(result),
        _ => None,
    }
}

/// What [`add_schema_reference`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaReferenceChange {
    /// The line was added to the file at this path.
    Added(String),
    /// A file holding just the line was created at this path.
    Created(String),
    /// The file could not be written: a warning, never a failed build.
    WriteFailed { path: String, error: String },
}

impl SchemaReferenceChange {
    /// The TS transformer's log line (an info line, or a warning and its hint for a failed write).
    pub fn message(&self, project_directory: &str) -> (String, Option<String>) {
        let shown = |path: &str| {
            let relative = node_path::relative(project_directory, path);
            if relative.is_empty() {
                path.to_owned()
            } else {
                relative
            }
        };
        match self {
            SchemaReferenceChange::Added(path) => (
                format!(
                    "Added a \"$schema\" line to {}, so editors list every option",
                    shown(path)
                ),
                None,
            ),
            SchemaReferenceChange::Created(path) => (
                format!(
                    "Created {} with a \"$schema\" line, so editors list every option",
                    shown(path)
                ),
                None,
            ),
            SchemaReferenceChange::WriteFailed { path, error } => (
                format!("Could not write a \"$schema\" line to {path}"),
                Some(error.clone()),
            ),
        }
    }
}

/// Gives a game's `flamework.config.json` a `$schema` line pointing at `schema_path` when it has none, or creates the
/// file with just that line when the game has none and its tsconfig is at the package root. Never for a package.
/// (Q-F5: whether rbxts-rs keeps writing it, and where the schema lives, is the user's call.)
pub fn add_schema_reference(
    project_directory: &str,
    root_directory: &str,
    entry: &JsObject,
    is_game: bool,
    schema_path: &str,
) -> Result<Option<SchemaReferenceChange>, ProjectError> {
    if !is_game {
        return Ok(None);
    }
    let config_path = locate_project_config(project_directory, root_directory, entry)?;
    let write = |path: &str, text: &str| match std::fs::write(path, text) {
        Ok(()) => None,
        Err(e) => Some(SchemaReferenceChange::WriteFailed {
            path: path.to_owned(),
            error: e.to_string(),
        }),
    };
    let Some(config_path) = config_path else {
        if !node_path::relative(
            &node_path::resolve(&[project_directory]),
            &node_path::resolve(&[root_directory]),
        )
        .is_empty()
        {
            return Ok(None);
        }
        let created = node_path::join(&[project_directory, PROJECT_CONFIG_NAME]);
        let mut reference = String::new();
        json::write_string(&mut reference, &get_schema_reference(&created, schema_path));
        let text = format!("{{\n\t\"$schema\": {reference}\n}}\n");
        return Ok(Some(
            write(&created, &text).unwrap_or(SchemaReferenceChange::Created(created)),
        ));
    };
    let Ok(text) = std::fs::read_to_string(&config_path) else {
        return Ok(None);
    };
    let Some(updated) =
        insert_schema_reference(&text, &get_schema_reference(&config_path, schema_path))
    else {
        return Ok(None);
    };
    Ok(Some(
        write(&config_path, &updated).unwrap_or(SchemaReferenceChange::Added(config_path)),
    ))
}
