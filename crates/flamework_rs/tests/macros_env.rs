//! F2 builds that need a process variable: an obfuscated build with a pinned seed (`FLAMEWORK_RS_FIXED_SEED`) and
//! the measuring switch (`FLAMEWORK_RS_DECLINE_UNPORTED`). One test in this binary, so setting them races nothing.

#![cfg(windows)]

mod support;

use support::fixture_macros::flamework_with_fixture_macros;
use support::project::{Project, assert_clean, luau};

const SEED: &str = "0f6a5e1c-4b7d-4c2e-9a3f-8d1b2c3e4f50";

#[test]
fn obfuscation_and_the_decline_switch() {
    // SAFETY: the only test of this binary; nothing reads the environment concurrently.
    unsafe {
        std::env::set_var("FLAMEWORK_RS_FIXED_SEED", SEED);
        std::env::set_var("FLAMEWORK_RS_DECLINE_UNPORTED", "1");
    }

    let Some(project) = Project::new("fixture_f2", "obfuscated") else { return };
    project.write(".env", "F2_FIXTURE_CHANNEL=beta\n");
    // Obfuscated ids hash a counter with the salt, which every clean build rotates: pinned, as the seed is.
    project.write(
        "flamework.config.json",
        "{ \"transformer\": { \"idGenerationMode\": \"obfuscated\", \"obfuscation\": true, \"salt\": \"f2-pinned-salt\" } }",
    );
    let first = project.build_with(flamework_with_fixture_macros());
    assert_clean(&first);
    let emit = luau(&first, "shared/emit.luau").to_owned();

    // Keys hashed in the "remotes" context; the number key kept.
    let seed = flamework_rs::uuid::Namespace::parse(SEED).unwrap();
    for key in ["playerJoined", "playerLeft"] {
        let hashed = seed.v5(&format!("remotes:{key}"));
        assert!(emit.contains(&format!("[\"{hashed}\"] = ")), "{key}: {emit}");
    }
    assert!(emit.contains("[3] = \"number key\","), "{emit}");
    // `Obfuscate` hashes under obfuscation.
    assert!(emit.contains(&format!("local name = obfuscated(\"{}\")", seed.v5("remotes:playerJoined"))), "{emit}");
    // The shuffled array holds the same elements.
    let order = emit.lines().find(|l| l.starts_with("local order = shuffled(")).expect("the order line");
    for element in ["\"a\"", "\"b\"", "\"c\""] {
        assert!(order.contains(element), "{order}");
    }
    // Callsite uuids come from the build seed.
    let caller = luau(&first, "shared/caller.luau");
    let uuid = flamework_rs::ids::callsite_uuid("f2-fixture", "src/shared/caller.ts", "atRoot", 9, Some(&seed));
    assert!(caller.contains(&uuid), "{caller}");

    // The same seed, the same bytes.
    let second = project.build_with(flamework_with_fixture_macros());
    assert_clean(&second);
    for (a, b) in first.files.iter().zip(&second.files) {
        assert_eq!(a.luau, b.luau, "{}", a.out_path);
    }

    // The switch: a guard is declined and listed, rather than an error.
    project.write(
        "src/shared/guarded.ts",
        "import { Flamework } from \"@flamework-experimental/core\";\n\nexport const guard = Flamework.createGuard<string>();\n",
    );
    let report = project.build_with(flamework_with_fixture_macros());
    assert_clean(&report);
    let warnings = report.warnings.join("\n");
    assert!(warnings.contains("(F3: 1)"), "{warnings}");
    assert!(luau(&report, "shared/guarded.luau").contains("Flamework.createGuard()"));
}
