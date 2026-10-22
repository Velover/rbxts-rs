//! `Plugin::setup` (the TS transformer's `TransformState` constructor and `setupBuildInfo`): the package, the config
//! and environment, the package build infos with the version check, the id settings, and the fresh `flamework.build`
//! with its keys created in DESIGN §10.4's order.
//!
//! Everything is read through the compiler ([`SetupCx::read_input`] / [`SetupCx::env`]), so every file and variable
//! read here is a build input.
//!
//! Clean builds only: every build starts from a new `flamework.build`, as a TS5 full build does (it rotates the salt
//! and the build seed). Reusing the previous one (incremental builds and watch rebuilds, with `refuseChangedInputs`
//! and the project version check) is TODO(S2).

use rbxts_plugin::{Cx, Fatal, SetupCx};

use crate::build_info::{self, BUILD_INFO_NAME, BuildInfo, CandidateScan};
use crate::config::{self, LoadedProjectConfig, TransformerConfig};
use crate::ids::{IdConfig, IdMode, Ids};
use crate::inputs::{Inputs, strip_bom};
use crate::json::{self, JsObject, JsValue};
use crate::packages::{CORE_PACKAGE, FLAMEWORK_SCOPE};
use crate::{ProjectError, node_path, uuid};

/// The test-only switch that pins the build seed (DESIGN §10.4), so obfuscated builds can be compared.
pub const FIXED_SEED_VAR: &str = "FLAMEWORK_RS_FIXED_SEED";

/// A test and measurement switch only: macro sites that a later milestone brings and that are errors by default
/// (`Modding.Generic<T, "guard">`, F3) are declined and listed in the build's warning instead, as the others are.
/// The build succeeds, but its output is not what Flamework emits: a declined site compiles as a plain call without
/// the values Flamework generates for it (a guard macro returns `nil`). **Such a build must not be run**; it exists
/// only to compare and time the rest of the output.
pub const DECLINE_UNPORTED_VAR: &str = "FLAMEWORK_RS_DECLINE_UNPORTED";

/// What every hook reads, made once per build (DESIGN §10.1). Read-only during the transform.
#[derive(Debug)]
pub struct FwShared {
    /// The tsconfig's directory (`currentDirectory`).
    pub project_dir: String,
    /// The directory of the package.json above it (`rootDirectory`): file ids and globs are relative to it.
    pub root_dir: String,
    pub package_name: String,
    /// A game (an unscoped package name), as opposed to a package.
    pub is_game: bool,
    /// `rootDirs`, else `[rootDir ?? currentDirectory]`: the project's own source roots.
    pub root_dirs: Vec<String>,
    /// The include folder (`--includePath`), absolute.
    pub include_dir: String,
    /// `transformer` options, with `idGenerationMode` and (for a package) `hashPrefix` filled in.
    pub config: TransformerConfig,
    /// The whole config file.
    pub loaded: LoadedProjectConfig,
    pub ids: Ids,
    /// This build's `flamework.build` as setup leaves it, with the package build infos as children.
    pub build_info: BuildInfo,
    /// The build seed, under obfuscation.
    pub build_seed: Option<uuid::Namespace>,
    /// The namespace of `Modding.Hash` values: the build seed under obfuscation, else a seed of this build's own
    /// (pinned by [`FIXED_SEED_VAR`]). The TS transformer made a random v4 per string and kept them in
    /// `flamework.build`'s `stringHashes`; derived ones need no table (DESIGN §10.3, §10.4). TODO(S2): an
    /// incremental build keeps it.
    pub hash_seed: uuid::Namespace,
    /// [`DECLINE_UNPORTED_VAR`] is set.
    pub decline_unported: bool,
    /// Whether the project has a Rojo project file (Flamework's `rojoResolver` is undefined without one).
    pub has_rojo: bool,
    /// Flamework's own path translator: roblox-ts's, with `.lua` outputs (`createPathTranslator`).
    pub path_translator: rbxts_project::path_translator::PathTranslator,
}

impl FwShared {
    /// `obfuscateText`: the text, or under obfuscation its derived hash (DESIGN §10.4).
    pub fn obfuscate_text(&self, text: &str, context: Option<&str>) -> String {
        match &self.build_seed {
            Some(seed) if self.config.obfuscation == Some(true) => {
                seed.v5(&format!("{}:{text}", context.unwrap_or("@")))
            }
            _ => text.to_owned(),
        }
    }

    /// `Flamework.env(name)` at transform time ([`crate::env::transform_env`]): what setup read from the `.env`
    /// files and the process, else the process environment through [`Cx::env`], recorded as an input of the file.
    pub fn env(&self, cx: &mut Cx<'_>, name: &str) -> Option<String> {
        crate::env::transform_env(&self.loaded.env, name, |name| cx.env(name))
    }

    /// `buildInfo.hashString`: a derived hash of `context:text` (`Modding.Hash`).
    pub fn hash_string(&self, text: &str, context: Option<&str>) -> String {
        self.hash_seed.v5(&format!("{}:{text}", context.unwrap_or("@")))
    }

    /// `getFileId`: a file relative to the package root, with forward slashes.
    pub fn file_id(&self, file_name: &str) -> String {
        node_path::relative(&self.root_dir, file_name).replace('\\', "/")
    }

    /// The `Reflect` import's specifier. Inside core itself, `Reflect` is imported by its relative module path.
    pub fn reflect_specifier(&self, file_name: &str) -> String {
        if self.package_name == CORE_PACKAGE {
            let module = node_path::join(&[&self.root_dir, "src", "reflect"]);
            let relative =
                node_path::relative(&node_path::dirname(file_name), &module).replace('\\', "/");
            // `"./" + relative || "."`: never empty.
            format!("./{relative}")
        } else {
            CORE_PACKAGE.to_owned()
        }
    }
}

/// [`Inputs`] over the compiler's recorded reads.
struct SetupInputs<'c, 'a> {
    cx: &'c mut SetupCx<'a>,
}

impl Inputs for SetupInputs<'_, '_> {
    fn read(&mut self, path: &str) -> Option<String> {
        self.cx.read_input(path).ok().flatten()
    }

    fn env(&mut self, name: &str) -> Option<String> {
        self.cx.env(name)
    }
}

/// A project mistake, said as the TS transformer says it: the first line as the message, the rest as the hint.
pub fn fatal(error: ProjectError) -> Fatal {
    let mut lines = error.0.splitn(2, '\n');
    let headline = lines.next().unwrap_or_default().to_owned();
    match lines.next() {
        Some(rest) => Fatal::new(headline).with_hint(rest),
        None => Fatal::new(headline),
    }
}

fn entry_object(cx: &SetupCx<'_>) -> JsObject {
    let Some(entry) = cx.tsconfig_entry() else {
        return JsObject::new();
    };
    // serde_json keeps the file's order (`preserve_order`); JSON text round-trips into JavaScript's order.
    let text = serde_json::to_string(entry).unwrap_or_else(|_| "{}".into());
    match json::parse(&text) {
        Ok(JsValue::Object(object)) => object,
        _ => JsObject::new(),
    }
}

/// Runs setup for a clean build (the persistent state is unused until incremental builds: TODO(S2)).
pub fn setup(cx: &mut SetupCx<'_>) -> Result<FwShared, Fatal> {
    let services = cx.project();
    let project_dir = services.data.project_path.clone();
    let include_dir = services.data.options.include_path.clone();
    let has_rojo = services.data.rojo_config_path.is_some();
    let path_translator = rbxts_project::path_translator::PathTranslator::new(
        services.path_translator.root_dir.clone(),
        services.path_translator.out_dir.clone(),
        None,
        services.path_translator.declaration,
        false,
    );
    let options = cx.options();
    let root_dir_option = options.root_dir.clone();
    let root_dirs_option = options.root_dirs.clone();
    let out_dir_option = options.out_dir.clone();
    let entry = entry_object(cx);
    let program = cx.program();

    // getPackageJson(currentDirectory)
    let (root_dir, package_name) = {
        let mut inputs = SetupInputs { cx };
        let package_json = build_info::find_package_json_with(&mut inputs, &project_dir, None)
            .ok_or_else(|| Fatal::new(format!("package.json not found in {project_dir}")))?;
        let text = inputs.read(&package_json).map(strip_bom).unwrap_or_default();
        let name = json::parse(&text)
            .ok()
            .and_then(|v| v.get("name").and_then(JsValue::as_str).map(|s| s.trim().to_owned()))
            .filter(|n| !n.is_empty())
            .ok_or_else(|| Fatal::new(format!("{package_json} has no package name")))?;
        (node_path::dirname(&package_json), name)
    };
    let is_game = !package_name.starts_with('@');

    // The `$schema` line the TS transformer adds to a game's flamework.config.json is not written: it would edit a
    // user's source file from a build (Q-F5, the user's call).

    let loaded = config::load_project_config_with(
        &mut SetupInputs { cx },
        &project_dir,
        &root_dir,
        &entry,
    )
    .map_err(fatal)?;
    if let Some(path) = &loaded.config_path {
        let shown = node_path::relative(&project_dir, path);
        cx.info(&format!(
            "Loaded project config from {}",
            if shown.is_empty() { path.as_str() } else { shown.as_str() }
        ));
    }

    let mut config = loaded.transformer();
    let build_info_path = node_path::join(&[&project_dir, BUILD_INFO_NAME]);
    let mut build_info = BuildInfo::new(&build_info_path);

    // The package build infos above the program's files, once per directory.
    {
        let mut directories: Vec<String> = Vec::new();
        for file in program.source_files() {
            let directory = node_path::dirname(file.get().file_name());
            if directories.last() != Some(&directory) {
                directories.push(directory);
            }
        }
        let mut inputs = SetupInputs { cx };
        let mut scan = CandidateScan::new();
        let candidates = scan.candidates_with(
            &mut inputs,
            directories.iter().map(String::as_str),
            &build_info_path,
        );
        let (infos, log) = build_info::load_package_build_infos_with(
            &mut inputs,
            &candidates,
            &project_dir,
            None,
        )
        .map_err(fatal)?;
        for line in log {
            cx.info(&line);
        }
        for info in infos {
            build_info.add_build_info(info);
        }
    }

    let mode = config.effective_id_mode();
    config.id_generation_mode = Some(mode);

    // In the TS transformer's order (DESIGN §10.4): inputs, idGenerationMode, identifierPrefix, then salt and
    // buildSeed, which TS adds at their first use and setup makes here; metadata comes last, in finish.
    build_info.set_config_input(&config::fingerprint_emit_config(&loaded));
    build_info.set_id_generation_mode(mode.as_str());

    let obfuscation = config.obfuscation == Some(true);
    if obfuscation && config.salt.is_some() {
        cx.warn(
            "transformer.salt is set with obfuscation on",
            Some(
                "A fixed salt gives every build the same obfuscated ids; leave it unset so each build gets fresh ones.",
            ),
        );
    }

    if is_game
        && !out_dir_option.is_empty()
        && node_path::is_path_descendant_of(&include_dir, &out_dir_option)
    {
        let include = node_path::relative(&project_dir, &include_dir).replace('\\', "/");
        cx.warn(
            &format!(
                "The include folder {} is inside outDir, which roblox-ts cleans of files that have no source",
                if include.is_empty() { "." } else { include.as_str() }
            ),
            Some(
                "include/flamework (globs.json, paths.json, config.json) is written again only after a build that succeeds, so a build that fails can leave the place without it. Move the include folder outside outDir.",
            ),
        );
    }

    if !is_game && config.hash_prefix.is_none() {
        config.hash_prefix = Some(package_name.clone());
    }
    build_info.set_identifier_prefix(config.hash_prefix.as_deref());

    if config.hash_prefix.as_deref().is_some_and(|p| p.starts_with('$'))
        && !package_name.starts_with(FLAMEWORK_SCOPE)
    {
        return Err(Fatal::new("The hashPrefix $ is used internally by Flamework"));
    }

    // transformer.plugins: JavaScript plugins cannot run here; macro types are registered in the build's main.rs
    // (Q-F8; the macro types themselves are F2).
    if !config.plugins.is_empty() {
        let names: Vec<String> = config.plugins.iter().map(|p| format!("'{}'", p.path)).collect();
        cx.warn(
            &format!(
                "transformer.plugins ({}) is ignored: rbxts-rs does not load JavaScript plugins",
                names.join(", ")
            ),
            Some("Register macro types in the build's main.rs instead (flamework_rs::Flamework::macro_type)."),
        );
    }

    let salt = match (&config.salt, mode) {
        (Some(salt), _) => salt.clone(),
        (None, IdMode::Full) => String::new(),
        (None, _) => build_info.salt(),
    };
    let build_seed = if obfuscation {
        match cx.env(FIXED_SEED_VAR) {
            Some(seed) => {
                build_info.set_build_seed(&seed).map_err(|e| {
                    Fatal::new(format!("{FIXED_SEED_VAR} is not a uuid: {e}"))
                })?;
                Some(build_info.build_seed())
            }
            None => Some(build_info.build_seed()),
        }
    } else {
        None
    };

    let hash_seed = match &build_seed {
        Some(seed) => seed.clone(),
        None => {
            let pinned = cx.env(FIXED_SEED_VAR);
            let text = pinned.unwrap_or_else(uuid::random_v4);
            uuid::Namespace::parse(&text).map_err(|e| Fatal::new(format!("{FIXED_SEED_VAR} is not a uuid: {e}")))?
        }
    };
    let decline_unported = cx.env(DECLINE_UNPORTED_VAR).is_some_and(|v| !v.is_empty() && v != "0" && v != "false");

    // The project's flamework.build came from another version: only a reused one can (TODO(S2)); a fresh one never.
    let out_relative = node_path::relative(&project_dir, &out_dir_option).replace('\\', "/");
    if let Err(lines) = build_info::check_project_version(&build_info, &out_relative) {
        let mut lines = lines.into_iter();
        let headline = lines.next().unwrap_or_default();
        return Err(Fatal::new(headline).with_hint(lines.collect::<Vec<_>>().join("\n")));
    }

    let src_dir = if root_dir_option.is_empty() { project_dir.clone() } else { root_dir_option };
    let root_dirs = root_dirs_option.unwrap_or_else(|| vec![src_dir]);

    let ids = Ids::new(IdConfig {
        mode,
        hash_prefix: config.hash_prefix.clone(),
        package_name: package_name.clone(),
        is_game,
        salt,
    });

    Ok(FwShared {
        project_dir,
        root_dir,
        package_name,
        is_game,
        root_dirs,
        include_dir,
        config,
        loaded,
        ids,
        build_info,
        build_seed,
        hash_seed,
        decline_unported,
        has_rojo,
        path_translator,
    })
}
