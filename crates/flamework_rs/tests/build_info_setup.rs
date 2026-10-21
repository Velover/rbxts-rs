//! The build-info setup that reads the disk: package build infos above the program's files, their refusals and the
//! version check, the derived string hashes, and late id assignment across file parts.

use std::path::PathBuf;

use flamework_rs::build_info::{
    BuildInfo, CandidateScan, check_project_version, load_package_build_infos,
    package_build_info_remedy,
};
use flamework_rs::ids::{Declaration, IdAnswer, IdConfig, IdMode, Ids};
use flamework_rs::{VERSION, uuid};

/// A tree in a folder of the test's own: `<temp>/fwrs-setup-<label>-<pid>/root`. Nothing is written outside the
/// owned folder, and the upward scans are given the owned folder as their ceiling, so stray files in the temp
/// directory cannot affect them.
struct Tree {
    owned: PathBuf,
    root: PathBuf,
}

impl Tree {
    fn new(label: &str) -> Self {
        let owned = std::env::temp_dir().join(format!("fwrs-setup-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&owned);
        let root = owned.join("root");
        std::fs::create_dir_all(&root).unwrap();
        Tree { owned, root }
    }

    fn ceiling(&self) -> String {
        self.owned.to_string_lossy().into_owned()
    }

    fn write(&self, relative: &str, text: &str) {
        let file = self.root.join(relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    fn path(&self, relative: &str) -> String {
        self.root
            .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR))
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.owned);
    }
}

fn build_info_text(version: &str, prefix: &str) -> String {
    format!(
        "{{\n\t\"version\": 1,\n\t\"flameworkVersion\": \"{version}\",\n\t\"identifiers\": {{}},\n\t\"identifierPrefix\": \"{prefix}\"\n}}"
    )
}

#[test]
fn package_build_infos_are_found_above_the_programs_files_once_each() {
    let t = Tree::new("scan");
    t.write("game/package.json", r#"{"name":"game"}"#);
    t.write("game/flamework.build", &build_info_text(VERSION, "game"));
    t.write(
        "game/node_modules/@flamework-experimental/core/package.json",
        r#"{"name":"@flamework-experimental/core"}"#,
    );
    t.write(
        "game/node_modules/@flamework-experimental/core/flamework.build",
        &build_info_text(VERSION, "$c"),
    );
    t.write(
        "game/node_modules/@s/lib/package.json",
        r#"{"name":"@s/lib"}"#,
    );
    t.write(
        "game/node_modules/@s/lib/flamework.build",
        &build_info_text("2.0.0-alpha.1", "@s/lib"),
    );
    // Five levels below the package: out of the upward scan's reach (the file and four parents).
    t.write(
        "game/node_modules/@s/deep/package.json",
        r#"{"name":"@s/deep"}"#,
    );
    t.write(
        "game/node_modules/@s/deep/flamework.build",
        &build_info_text(VERSION, "@s/deep"),
    );

    let dirs = [
        t.path("game/src/server"),
        t.path("game/node_modules/@flamework-experimental/core/out"),
        t.path("game/node_modules/@flamework-experimental/core/out/sub"),
        t.path("game/node_modules/@s/lib/out"),
        t.path("game/node_modules/@s/deep/a/b/c/d/e"),
    ];
    let owned = t.ceiling();
    let mut scan = CandidateScan::with_ceiling(&owned);
    let ceiling = Some(owned.as_str());
    let own = t.path("game/flamework.build");
    let candidates = scan.candidates(dirs.iter().map(String::as_str), &own.to_uppercase());
    assert_eq!(
        candidates,
        [
            t.path("game/node_modules/@flamework-experimental/core/flamework.build"),
            t.path("game/node_modules/@s/lib/flamework.build"),
        ],
        "the game's own is left out (compared without case on Windows), the core one is found once"
    );

    // A package of another scope may come from any 2.x; Flamework's own may not.
    let (infos, log) = load_package_build_infos(&candidates, &t.path("game"), ceiling).unwrap();
    assert_eq!(infos.len(), 2);
    assert_eq!(
        log[0],
        "Loaded buildInfo at node_modules/@flamework-experimental/core/flamework.build, next id: 1"
    );
    t.write(
        "game/node_modules/@flamework-experimental/core/flamework.build",
        &build_info_text("2.0.0-alpha.7", "$c"),
    );
    let error = load_package_build_infos(&candidates, &t.path("game"), ceiling).unwrap_err();
    assert!(error.0.contains("  @flamework-experimental/core 2.0.0-alpha.7 (node_modules/@flamework-experimental/core/flamework.build)"), "{error}");

    // A package build info that cannot be used names its package.
    t.write("game/node_modules/@s/lib/flamework.build", "{");
    let error = load_package_build_infos(&candidates[1..], &t.path("game"), ceiling).unwrap_err();
    assert_eq!(
        error.0,
        format!(
            "Flamework cannot use node_modules/@s/lib/flamework.build: it is not valid JSON (Expected property name or '}}' in JSON at position 1 (line 1 column 2)).\n{}",
            package_build_info_remedy(&candidates[1], ceiling)[0]
        )
    );
    assert!(error.0.ends_with("It came with the package @s/lib, which wrote it when it was built, and this build reads it for the ids of that package's classes. Reinstall the package, or build it again if it is your own."));

    // The project's own: in the directory, else at the package root.
    let own_info = BuildInfo::from_directory(&t.path("game/src"), &[], ceiling)
        .unwrap()
        .unwrap();
    assert_eq!(own_info.identifier_prefix(), Some("game"));

    // The scans stop at the ceiling: a package.json above it is not found.
    t.write("lone/x/file.ts", "");
    assert_eq!(
        flamework_rs::build_info::find_package_json(&t.path("lone/x"), ceiling),
        None
    );
    std::fs::write(t.owned.join("package.json"), r#"{"name":"owned"}"#).unwrap();
    assert_eq!(
        flamework_rs::build_info::package_name_of(&t.path("lone/x/file.ts"), ceiling).as_deref(),
        Some("owned"),
        "the ceiling itself is still looked in"
    );
}

#[test]
fn the_project_version_check_names_both_versions() {
    let mut info = BuildInfo::new("x");
    assert!(check_project_version(&info, "out").is_ok());
    info = BuildInfo::from_value(
        "x",
        flamework_rs::json::parse(&build_info_text("2.0.0-alpha.1", "p")).unwrap(),
    );
    assert_eq!(
        check_project_version(&info, "out").unwrap_err(),
        [
            "Project was compiled on different version of Flamework.".to_owned(),
            "Please recompile by deleting the out directory".to_owned(),
            format!("Current Flamework Version: {VERSION}"),
            "Previous Flamework Version: 2.0.0-alpha.1".to_owned(),
        ]
    );
}

#[test]
fn string_hashes_derive_from_the_build_seed() {
    let mut info = BuildInfo::new("x");
    info.set_build_seed("6f4c1d2e-8b3a-4e5f-9c7d-2a1b0e9f8d7c")
        .unwrap();
    assert_eq!(
        info.set_build_seed("not a uuid"),
        Err("Invalid UUID".to_owned())
    );
    let a = info.hash_string("src/glob/**", Some("addPaths"));
    assert_eq!(
        a,
        uuid::v5(
            "addPaths:src/glob/**",
            "6f4c1d2e-8b3a-4e5f-9c7d-2a1b0e9f8d7c"
        )
        .unwrap()
    );
    assert_eq!(
        info.hash_string("src/glob/**", Some("addPaths")),
        a,
        "the same within a build"
    );
    assert_eq!(
        info.hash_string("x", None),
        uuid::v5("@:x", "6f4c1d2e-8b3a-4e5f-9c7d-2a1b0e9f8d7c").unwrap(),
        "the context defaults to @"
    );
    assert!(!info.to_json().contains("stringHashes"), "no table is kept");

    let mut fresh = BuildInfo::new("y");
    let seed = fresh.build_seed();
    assert!(uuid::validate(seed.as_str()));
    assert_ne!(
        fresh.hash_string("src/glob/**", Some("addPaths")),
        a,
        "a clean build rotates them"
    );
    assert_eq!(fresh.salt().len(), 128);

    // A seed that is not a uuid: refused when read from a file, dropped from a value, never a panic.
    let bad =
        r#"{ "version": 1, "flameworkVersion": "x", "identifiers": {}, "buildSeed": "nope" }"#;
    assert_eq!(
        flamework_rs::build_info::find_problem(Some(bad)).as_deref(),
        Some("its buildSeed \"nope\" is not a uuid")
    );
    let mut from_value = BuildInfo::from_value("z", flamework_rs::json::parse(bad).unwrap());
    let replaced = from_value.build_seed();
    assert!(uuid::validate(replaced.as_str()));
    let _ = from_value.hash_string("a", None);
    assert_eq!(
        flamework_rs::ids::callsite_uuid("p", "src/a.ts", "A", 3, Some(&replaced)),
        replaced.v5("p:src/a.ts@A+3")
    );
}

/// Ids are the same whatever order the files were transformed in, as long as the parts are merged in program order.
#[test]
fn late_ids_follow_program_order_not_transform_order() {
    let ids = Ids::new(IdConfig {
        mode: IdMode::Short,
        hash_prefix: None,
        package_name: "game".into(),
        is_game: true,
        salt: "salt".into(),
    });
    let decl = |file: &str, name: &str| Declaration {
        internal_id: format!("game:out/{file}@{name}"),
        is_package: false,
        name: name.into(),
        source_file: format!("C:\\p\\src\\{file}.ts"),
        file_name: file.into(),
    };
    let files = [
        vec![decl("a", "A"), decl("a", "B")],
        vec![decl("b", "C"), decl("a", "A")],
    ];

    let run = |transform_order: &[usize]| {
        let mut info = BuildInfo::new("x");
        let mut parts: Vec<(usize, Vec<_>)> = Vec::new();
        let mut answers: Vec<(usize, Vec<IdAnswer>)> = Vec::new();
        for &f in transform_order {
            let mut part = Vec::new();
            let mut file_answers = Vec::new();
            for d in &files[f] {
                let (answer, new) = ids.request(&info, d);
                if let Some(new) = new {
                    part.push(new);
                }
                file_answers.push(answer);
            }
            parts.push((f, part));
            answers.push((f, file_answers));
        }
        parts.sort_by_key(|(f, _)| *f);
        let resolved = ids.assign(&mut info, parts.iter().map(|(_, p)| p.as_slice()));
        answers.sort_by_key(|(f, _)| *f);
        let ids: Vec<String> = answers
            .into_iter()
            .flat_map(|(_, a)| a)
            .map(|a| match a {
                IdAnswer::Now(id) => id,
                IdAnswer::Late(key) => resolved[&key].clone(),
            })
            .collect();
        (ids, info.to_json())
    };
    let forward = run(&[0, 1]);
    assert_eq!(run(&[1, 0]), forward);
    assert!(forward.0[0].starts_with("a@A{") && forward.0[3] == forward.0[0]);
}
