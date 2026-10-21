//! Table-driven parity with the node libraries and the TS transformer. The tables in `tests/data` come from
//! `tests/data/gen/generate.cjs`, run against the Flamework monorepo's `node_modules` and its built transformer.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use flamework_rs::build_info::{BuildInfo, GlobUse, MixedVersion, PathUse, refuse_mixed_versions};
use flamework_rs::config;
use flamework_rs::env::{self, Env};
use flamework_rs::hashids::Hashids;
use flamework_rs::ids::{self, Declaration, IdAnswer, IdConfig, IdMode, Ids};
use flamework_rs::json::{self, JsObject, JsValue};
use flamework_rs::{glob, sha, uuid};

fn table(name: &str) -> JsValue {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    json::parse(&std::fs::read_to_string(&path).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn s(v: &JsValue, key: &str) -> String {
    v.get(key)
        .and_then(JsValue::as_str)
        .unwrap_or_else(|| panic!("no string {key} in {}", v.to_json()))
        .to_owned()
}

fn arr<'a>(v: &'a JsValue, key: &str) -> &'a Vec<JsValue> {
    v.get(key)
        .and_then(JsValue::as_array)
        .unwrap_or_else(|| panic!("no array {key}"))
}

/// `[[k, v], ...]` as an environment.
fn env_of(v: &JsValue) -> Env {
    v.as_array()
        .unwrap()
        .iter()
        .map(|kv| {
            let kv = kv.as_array().unwrap();
            (
                kv[0].as_str().unwrap().to_owned(),
                kv[1].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn entries_json(env: &Env) -> String {
    JsValue::Array(
        env.iter()
            .map(|(k, v)| {
                JsValue::Array(vec![JsValue::String(k.into()), JsValue::String(v.into())])
            })
            .collect(),
    )
    .to_json()
}

/// A folder of the test's own, `<temp>/fwrs-<label>-<pid>-<n>`, with the test's tree in its `root` child: a case
/// that writes above its root (`../flamework.config.json`) writes into the owned folder, never into the temp
/// directory itself, and the whole owned folder is removed on drop.
struct TempDir {
    owned: PathBuf,
    root: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let owned = std::env::temp_dir().join(format!(
            "fwrs-{label}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&owned);
        let root = owned.join("root");
        std::fs::create_dir_all(&root).unwrap();
        TempDir { owned, root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn str(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let file = self.root.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
        file
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.owned);
    }
}

/// Collects mismatches; fails with the first few after printing the count.
struct Check {
    name: &'static str,
    total: usize,
    failures: Vec<String>,
}

impl Check {
    fn new(name: &'static str) -> Self {
        Check {
            name,
            total: 0,
            failures: Vec::new(),
        }
    }

    fn eq(&mut self, what: impl FnOnce() -> String, expected: &str, actual: &str) {
        self.total += 1;
        if expected != actual {
            self.failures.push(format!(
                "{}\n    expected: {expected:?}\n    actual:   {actual:?}",
                what()
            ));
        }
    }

    fn done(self) {
        println!(
            "[parity] {}: {}/{} rows equal",
            self.name,
            self.total - self.failures.len(),
            self.total
        );
        if !self.failures.is_empty() {
            let shown: Vec<&String> = self.failures.iter().take(25).collect();
            panic!(
                "{}: {} of {} rows differ:\n{}",
                self.name,
                self.failures.len(),
                self.total,
                shown
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }
}

#[test]
fn hashids_matches_npm_hashids() {
    let t = table("hashids.json");
    let mut c = Check::new("hashids");
    for series in arr(&t, "series") {
        let salt = s(series, "salt");
        let min = series.get("minLength").unwrap().as_f64().unwrap() as usize;
        let start = series.get("start").unwrap().as_f64().unwrap() as u64;
        let h = Hashids::new(&salt, min);
        for (i, expected) in arr(series, "hashes").iter().enumerate() {
            let n = start + i as u64;
            c.eq(
                || format!("salt {salt:?} n {n}"),
                expected.as_str().unwrap(),
                &h.encode_one(n),
            );
        }
    }
    for row in arr(&t, "extra") {
        let salt = s(row, "salt");
        let min = row.get("minLength").unwrap().as_f64().unwrap() as usize;
        let numbers: Vec<u64> = arr(row, "numbers")
            .iter()
            .map(|n| n.as_f64().unwrap() as u64)
            .collect();
        c.eq(
            || format!("salt {salt:?} min {min} {numbers:?}"),
            &s(row, "hash"),
            &Hashids::new(&salt, min).encode(&numbers),
        );
    }
    c.done();
}

#[test]
fn uuid_v5_matches_npm_uuid() {
    let t = table("uuid.json");
    let mut c = Check::new("uuid v5");
    for row in arr(&t, "rows") {
        let (name, namespace) = (s(row, "name"), s(row, "namespace"));
        let actual = uuid::v5(&name, &namespace).unwrap_or_else(|e| e);
        c.eq(
            || format!("{name:.40?} in {namespace}"),
            &s(row, "uuid"),
            &actual,
        );
    }
    for row in arr(&t, "invalid") {
        let namespace = s(row, "namespace");
        c.eq(
            || format!("invalid {namespace:?}"),
            &s(row, "error"),
            &uuid::v5("a", &namespace).unwrap_err(),
        );
    }
    c.done();
}

#[test]
fn hashes_match_node_crypto() {
    let t = table("hashes.json");
    let mut c = Check::new("sha1/sha256/layoutHash");
    for row in arr(&t, "rows") {
        let input = s(row, "input");
        c.eq(
            || format!("sha1 {input:.30?}"),
            &s(row, "sha1"),
            &sha::hex(&sha::sha1(input.as_bytes())),
        );
        c.eq(
            || format!("sha256 {input:.30?}"),
            &s(row, "sha256"),
            &sha::hex(&sha::sha256(input.as_bytes())),
        );
    }
    for row in arr(&t, "layouts") {
        let d = s(row, "description");
        let expected = row.get("hash").unwrap().as_f64().unwrap() as u32;
        c.eq(
            || format!("layoutHash {d:.30?}"),
            &expected.to_string(),
            &sha::layout_hash(&d).to_string(),
        );
    }
    c.done();
}

#[test]
fn env_files_parse_as_the_transformer_parses_them() {
    let t = table("env.json");
    let mut c = Check::new("env");
    for row in arr(&t, "rows") {
        let text = s(row, "text");
        let expected = row.get("entries").unwrap().to_json();
        c.eq(
            || format!("parseEnvFile {text:?}"),
            &expected,
            &entries_json(&env::parse_env_file(&text)),
        );
    }
    let load = t.get("loadEnv").unwrap();
    let dir = TempDir::new("env");
    for (name, text) in load.get("files").unwrap().as_object().unwrap().iter() {
        dir.write(name, text.as_str().unwrap());
    }
    for row in arr(load, "loads") {
        let process = env_of(row.get("processEnv").unwrap());
        let expected = row.get("entries").unwrap().to_json();
        c.eq(
            || format!("loadEnv {}", row.get("processEnv").unwrap().to_json()),
            &expected,
            &entries_json(&env::load_env(dir.path(), &process)),
        );
    }
    c.done();
}

#[test]
fn config_files_read_as_the_transformer_reads_them() {
    let t = table("config.json");
    let dir = TempDir::new("config");
    let file = dir.path().join("flamework.config.json");
    let file_str = file.to_string_lossy().into_owned();
    let mut c = Check::new("readProjectConfig");
    for case in arr(&t, "cases") {
        let name = s(case, "name");
        std::fs::write(&file, s(case, "text")).unwrap();
        let env = env_of(case.get("env").unwrap());
        let actual = match config::read_project_config(&file_str, &env) {
            Ok(v) => format!("ok {}", v.to_json()),
            Err(e) => format!("error {}", e.message().replace(&file_str, "<FILE>")),
        };
        let expected = match case.get("error") {
            Some(e) => format!("error {}", e.as_str().unwrap()),
            None => format!("ok {}", s(case, "result")),
        };
        c.eq(|| name.clone(), &expected, &actual);
    }
    c.done();
}

#[test]
fn config_files_are_found_and_tsconfig_entries_checked_as_the_transformer_does() {
    let t = table("entry.json");
    let mut c = Check::new("loadProjectConfig");
    for case in arr(&t, "cases") {
        let name = s(case, "name");
        let root = TempDir::new("entry");
        root.write("package.json", r#"{"name":"config-test"}"#);
        std::fs::create_dir_all(root.path().join("places/a")).unwrap();
        for file in arr(case, "files") {
            let file = file.as_array().unwrap();
            root.write(file[0].as_str().unwrap(), file[1].as_str().unwrap());
        }
        let root_str = root.str();
        let place = root
            .path()
            .join("places")
            .join("a")
            .to_string_lossy()
            .into_owned();
        let project = if s(case, "projectDir") == "root" {
            root_str.clone()
        } else {
            place
        };
        let entry = match json::parse(&s(case, "entry")).unwrap() {
            JsValue::Object(o) => o,
            _ => JsObject::new(),
        };
        let env = env_of(case.get("env").unwrap());
        let actual = match config::load_project_config(&project, &root_str, &entry, &env) {
            Ok(loaded) => {
                let mut o = JsObject::new();
                o.insert(
                    "configPath",
                    loaded.config_path.map_or(JsValue::Null, |p| {
                        JsValue::String(p.replace(&root_str, "<ROOT>"))
                    }),
                );
                o.insert("config", JsValue::String(loaded.config.to_json()));
                o.insert("project", JsValue::String(loaded.project.to_json()));
                o.insert("env", json::parse(&entries_json(&loaded.env)).unwrap());
                format!("ok {}", JsValue::Object(o).to_json())
            }
            Err(e) => format!("error {}", e.message().replace(&root_str, "<ROOT>")),
        };
        let expected = match case.get("error") {
            Some(e) => format!("error {}", e.as_str().unwrap()),
            None => format!("ok {}", case.get("result").unwrap().to_json()),
        };
        c.eq(|| name.clone(), &expected, &actual);
    }
    c.done();
}

#[test]
fn schema_lines_moved_keys_and_runtime_sections_match() {
    let t = table("schema-ref.json");
    let mut c = Check::new("$schema/moved/runtime");
    for row in arr(&t, "inserts") {
        let text = s(row, "text");
        let expected = row.get("result").unwrap().to_json();
        let actual = config::insert_schema_reference(&text, "./s.json")
            .map_or(JsValue::Null, JsValue::String)
            .to_json();
        c.eq(
            || format!("insertSchemaReference {text:?}"),
            &expected,
            &actual,
        );
    }
    for row in arr(&t, "refs") {
        let (cfg, schema) = (s(row, "config"), s(row, "schema"));
        c.eq(
            || format!("getSchemaReference {cfg} {schema}"),
            &s(row, "result"),
            &config::get_schema_reference(&cfg, &schema),
        );
    }
    for row in arr(&t, "moved") {
        let value = json::parse(&s(row, "config")).unwrap();
        let actual = JsValue::Array(
            config::describe_moved_keys(&value)
                .into_iter()
                .map(JsValue::String)
                .collect(),
        )
        .to_json();
        c.eq(
            || format!("describeMovedKeys {}", s(row, "config")),
            &row.get("lines").unwrap().to_json(),
            &actual,
        );
    }
    for row in arr(&t, "runtime") {
        let project = json::parse(&s(row, "project")).unwrap();
        let actual = config::get_runtime_config(&project)
            .unwrap_or(JsValue::Null)
            .to_json();
        c.eq(
            || format!("getRuntimeConfig {}", s(row, "project")),
            &s(row, "runtime"),
            &actual,
        );
    }
    c.done();
}

fn glob_use(v: &JsValue) -> GlobUse {
    GlobUse {
        glob: s(v, "glob"),
        text: s(v, "text"),
        macro_name: s(v, "macro"),
        line: v.get("line").unwrap().as_f64().unwrap() as u32,
        column: v.get("column").unwrap().as_f64().unwrap() as u32,
    }
}

fn path_use(v: &JsValue) -> PathUse {
    PathUse {
        path: s(v, "path"),
        macro_name: s(v, "macro"),
        line: v.get("line").unwrap().as_f64().unwrap() as u32,
        column: v.get("column").unwrap().as_f64().unwrap() as u32,
        raises: v.get("raises").and_then(JsValue::as_bool).unwrap_or(false),
    }
}

#[test]
fn build_infos_read_and_write_as_the_transformer_does() {
    let t = table("buildinfo.json");
    let mut c = Check::new("flamework.build");
    let dir = TempDir::new("buildinfo");
    let file = dir
        .path()
        .join("broken.build")
        .to_string_lossy()
        .into_owned();
    for row in arr(&t, "problems") {
        let text = s(row, "text");
        std::fs::write(&file, &text).unwrap();
        let actual = match BuildInfo::from_path(&file, "broken.build", &["Do this.".to_owned()]) {
            Ok(_) => JsValue::Null,
            Err(e) => JsValue::String(e.0),
        };
        c.eq(
            || format!("fromPath {text:.60?}"),
            &row.get("error").unwrap().to_json(),
            &actual.to_json(),
        );
    }
    for seq in arr(&t, "sequences") {
        let name = s(seq, "name");
        let mut info = BuildInfo::new("seq.build");
        let mut results = Vec::new();
        for op in arr(seq, "ops") {
            let op = op.as_array().unwrap();
            let arg = |i: usize| {
                op.get(i)
                    .and_then(JsValue::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let result = match op[0].as_str().unwrap() {
                "setIdGenerationMode" => info
                    .set_id_generation_mode(&arg(1))
                    .map_or(JsValue::Null, JsValue::String),
                "setIdentifierPrefix" => {
                    info.set_identifier_prefix(op.get(1).and_then(JsValue::as_str));
                    JsValue::Null
                }
                "addIdentifier" => {
                    info.add_identifier(&arg(1), &arg(2)).unwrap();
                    JsValue::Null
                }
                "getLatestId" => JsValue::Number(info.latest_id() as f64),
                "getIdentifierFromInternal" => info
                    .identifier_from_internal(&arg(1))
                    .map_or(JsValue::Null, JsValue::String),
                "addGlob" => {
                    let u = op.get(3).map(glob_use);
                    info.add_glob(&arg(1), &arg(2), u.as_ref());
                    JsValue::Null
                }
                "addPathUse" => {
                    info.add_path_use(&arg(1), &path_use(&op[2]));
                    JsValue::Null
                }
                "setMetadata" => {
                    info.set_metadata(&arg(1), op[2].clone());
                    JsValue::Null
                }
                "invalidateGlobs" => {
                    info.invalidate_globs(&arg(1));
                    JsValue::Null
                }
                "invalidatePathUses" => {
                    info.invalidate_path_uses(&arg(1));
                    JsValue::Null
                }
                other => panic!("unknown op {other}"),
            };
            results.push(result);
        }
        c.eq(
            || format!("{name} results"),
            &seq.get("results").unwrap().to_json(),
            &JsValue::Array(results).to_json(),
        );
        c.eq(|| format!("{name} text"), &s(seq, "text"), &info.to_json());
    }
    for row in arr(&t, "mixed") {
        let packages: Vec<MixedVersion> = arr(row, "packages")
            .iter()
            .map(|p| MixedVersion {
                name: s(p, "name"),
                version: s(p, "version"),
                file: s(p, "file"),
            })
            .collect();
        let actual = refuse_mixed_versions(&packages)
            .err()
            .map_or(JsValue::Null, |e| JsValue::String(e.0));
        c.eq(
            || format!("refuseMixedVersions {}", packages.len()),
            &row.get("error").unwrap().to_json(),
            &actual.to_json(),
        );
    }
    let real_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/buildinfo");
    for row in arr(&t, "real") {
        let name = s(row, "name");
        let path = real_dir.join(&name);
        let text = std::fs::read_to_string(&path).unwrap();
        let info = BuildInfo::from_path(&path.to_string_lossy(), &name, &[]).unwrap();
        let rewrites = row.get("rewrites").unwrap().as_bool().unwrap();
        c.eq(
            || format!("{name} rewrites as TS does"),
            &rewrites.to_string(),
            &(info.to_json() == text).to_string(),
        );
        c.eq(|| format!("{name} bytes"), &text, &info.to_json());
    }
    c.done();
}

#[test]
fn ids_match_the_transformer() {
    let t = table("ids.json");
    let mut c = Check::new("ids");
    for row in arr(&t, "formats") {
        let id = s(row, "internalId");
        let prefix = row.get("prefix").and_then(JsValue::as_str);
        c.eq(
            || format!("formatInternalid {id:?} {prefix:?}"),
            &s(row, "result"),
            &ids::format_internal_id(&id, prefix),
        );
    }

    let declarations: Vec<Declaration> = arr(&t, "declarations")
        .iter()
        .map(|d| Declaration {
            internal_id: s(d, "internalId"),
            is_package: d
                .get("isPackage")
                .and_then(JsValue::as_bool)
                .unwrap_or(false),
            name: s(d, "fullName"),
            source_file: s(d, "fileName"),
            file_name: flamework_rs::node_path::win32::parse_name(&s(d, "fileName")),
        })
        .collect();
    for scenario in arr(&t, "scenarios") {
        let mode = IdMode::parse(&s(scenario, "mode")).unwrap();
        let hash_prefix = scenario
            .get("hashPrefix")
            .and_then(JsValue::as_str)
            .map(str::to_owned);
        let config = IdConfig {
            mode,
            hash_prefix: hash_prefix.clone(),
            package_name: s(scenario, "packageName"),
            is_game: scenario.get("isGame").unwrap().as_bool().unwrap(),
            salt: s(scenario, "salt"),
        };
        let label = format!(
            "{} game={} prefix={:?} salt={:.8}",
            mode.as_str(),
            config.is_game,
            hash_prefix,
            config.salt
        );
        let ids = Ids::new(config);
        let mut info = BuildInfo::new("C:\\p\\flamework.build");
        info.set_id_generation_mode(mode.as_str());
        info.set_identifier_prefix(hash_prefix.as_deref());
        let mut other = BuildInfo::new("C:\\p\\node_modules\\@s\\other\\flamework.build");
        other.set_identifier_prefix(Some("@s/other"));
        info.add_build_info(other);

        // One file part: every request in order, new ids recorded once per internal id.
        let mut answers = Vec::new();
        let mut part: Vec<ids::NewId> = Vec::new();
        for d in &declarations {
            let (answer, new) = ids.request(&info, d);
            if let Some(new) = new
                && !part.iter().any(|n| n.internal_id == new.internal_id)
            {
                part.push(new);
            }
            answers.push(answer);
        }
        let resolved = ids.assign(&mut info, [part.as_slice()]);
        let actual: Vec<JsValue> = answers
            .into_iter()
            .map(|a| match a {
                IdAnswer::Now(id) => JsValue::String(id),
                IdAnswer::Late(key) => JsValue::String(resolved[&key].clone()),
            })
            .collect();
        c.eq(
            || format!("{label} ids"),
            &scenario.get("ids").unwrap().to_json(),
            &JsValue::Array(actual).to_json(),
        );
        let table: Vec<JsValue> = info
            .data()
            .get("identifiers")
            .unwrap()
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| JsValue::Array(vec![JsValue::String(k.clone()), v.clone()]))
            .collect();
        c.eq(
            || format!("{label} table"),
            &scenario.get("identifiers").unwrap().to_json(),
            &JsValue::Array(table).to_json(),
        );
    }

    for row in arr(&t, "internals") {
        let root_dirs: Vec<String> = arr(row, "rootDirs")
            .iter()
            .map(|d| d.as_str().unwrap().to_owned())
            .collect();
        let roots: Vec<&str> = root_dirs.iter().map(String::as_str).collect();
        let file = s(row, "filePath");
        let (package, name) = (s(row, "packageName"), s(row, "fullName"));
        let (is_package, internal) = if ids::is_project_file(&file, &roots) {
            (
                false,
                ids::project_internal_id(
                    &package,
                    &s(row, "currentDirectory"),
                    &s(row, "outputPath"),
                    &name,
                ),
            )
        } else {
            (
                true,
                ids::package_internal_id(&package, &s(row, "packageDir"), &file, &name),
            )
        };
        let result = row.get("result").unwrap();
        c.eq(
            || format!("internal id {file}"),
            &s(result, "internalId"),
            &internal,
        );
        c.eq(
            || format!("isPackage {file}"),
            &result
                .get("isPackage")
                .unwrap()
                .as_bool()
                .unwrap()
                .to_string(),
            &is_package.to_string(),
        );
    }
    c.done();
}

#[test]
fn globs_match_node_glob() {
    let t = table("glob.json");
    let root = TempDir::new("glob");
    for file in arr(&t, "tree") {
        root.write(file.as_str().unwrap(), "");
    }
    for d in arr(&t, "emptyDirs") {
        std::fs::create_dir_all(root.path().join(d.as_str().unwrap())).unwrap();
    }
    let prefix = root.str().replace('\\', "/");
    let mut c = Check::new("glob");
    for row in arr(&t, "rows") {
        let pattern = s(row, "pattern");
        let expected = row
            .get("results")
            .map(JsValue::to_json)
            .unwrap_or_else(|| format!("error {}", s(row, "error")));
        let actual: Vec<JsValue> = glob::glob_sync(&pattern, root.path())
            .into_iter()
            .map(|r| JsValue::String(r.replace(&prefix, "<ROOT>")))
            .collect();
        c.eq(
            || format!("glob {pattern:?}"),
            &expected,
            &JsValue::Array(actual).to_json(),
        );
    }
    c.done();
}

#[test]
fn brace_expansion_matches_brace_expansion() {
    let t = table("braces.json");
    let mut c = Check::new("brace expansion");
    for row in arr(&t, "rows") {
        let input = s(row, "input");
        let out = glob::brace_expand(&input);
        let count = row.get("count").unwrap().as_f64().unwrap() as usize;
        c.eq(
            || format!("{input} count"),
            &count.to_string(),
            &out.len().to_string(),
        );
        let strings = |v: &[String]| {
            JsValue::Array(v.iter().cloned().map(JsValue::String).collect()).to_json()
        };
        let head = &out[..out.len().min(20)];
        let tail = &out[out.len().saturating_sub(5)..];
        c.eq(
            || format!("{input} head"),
            &row.get("head").unwrap().to_json(),
            &strings(head),
        );
        c.eq(
            || format!("{input} tail"),
            &row.get("tail").unwrap().to_json(),
            &strings(tail),
        );
    }
    c.done();
}

/// Makes a link at `at` to `target`: a junction, a directory symbolic link or a file symbolic link.
fn make_link(target: &Path, at: &Path, kind: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        match kind {
            "junction" => {
                let status = std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(at)
                    .arg(target)
                    .stdout(std::process::Stdio::null())
                    .status()?;
                if status.success() {
                    Ok(())
                } else {
                    Err(std::io::Error::other(format!("mklink /J failed: {status}")))
                }
            }
            "dir" => std::os::windows::fs::symlink_dir(target, at),
            _ => std::os::windows::fs::symlink_file(target, at),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = kind;
        std::os::unix::fs::symlink(target, at)
    }
}

/// glob 9's rules for `**` and links (junctions and symbolic links): a folder linked into `src` has to load. The
/// tree is the validator's; it needs links, so set `FLAMEWORK_RS_SKIP_LINK_TESTS=1` on a machine that cannot make
/// symbolic links (Windows without developer mode).
#[test]
fn globs_over_links_match_node_glob() {
    let t = table("glob-links.json");
    let root = TempDir::new("globlinks");
    for file in arr(&t, "files") {
        root.write(file.as_str().unwrap(), "");
    }
    for link in arr(&t, "links") {
        let (target, at, kind) = (s(link, "target"), s(link, "at"), s(link, "type"));
        let native = |p: &str| {
            root.path()
                .join(p.replace('/', std::path::MAIN_SEPARATOR_STR))
        };
        if let Err(e) = make_link(&native(&target), &native(&at), &kind) {
            if std::env::var("FLAMEWORK_RS_SKIP_LINK_TESTS").is_ok() {
                println!("[parity] glob over links: skipped ({kind} link {at}: {e})");
                return;
            }
            panic!(
                "cannot make the {kind} link {at} -> {target}: {e} (FLAMEWORK_RS_SKIP_LINK_TESTS=1 skips this test)"
            );
        }
    }
    let prefix = root.str().replace('\\', "/");
    let mut c = Check::new("glob over links");
    for row in arr(&t, "rows") {
        let pattern = s(row, "pattern");
        let expected = row
            .get("results")
            .map(JsValue::to_json)
            .unwrap_or_else(|| format!("error {}", s(row, "error")));
        let actual: Vec<JsValue> = glob::glob_sync(&pattern, root.path())
            .into_iter()
            .map(|r| JsValue::String(r.replace(&prefix, "<ROOT>")))
            .collect();
        c.eq(
            || format!("glob {pattern:?}"),
            &expected,
            &JsValue::Array(actual).to_json(),
        );
    }
    c.done();
}
