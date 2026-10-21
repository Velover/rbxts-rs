//! Deep nesting never aborts the process: JSON and build infos read, write, copy and drop 3,000 levels on a 1 MiB
//! stack, as V8 reads them; a config nested deeper than `jsonc::MAX_DEPTH` is refused with an error, as TypeScript
//! (which runs out of stack around a thousand levels) refuses it.

use flamework_rs::build_info::{BuildInfo, find_problem};
use flamework_rs::env::Env;
use flamework_rs::{config, json, jsonc};

/// Runs `f` on a thread with a 1 MiB stack, the Windows main thread's.
fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

fn arrays(depth: usize) -> String {
    format!("{}1{}", "[".repeat(depth), "]".repeat(depth))
}

fn objects(depth: usize) -> String {
    format!("{}1{}", "{\"a\":".repeat(depth), "}".repeat(depth))
}

#[test]
fn json_reads_and_writes_3000_levels() {
    on_small_stack(|| {
        for text in [arrays(3000), objects(3000)] {
            let value = json::parse(&text).unwrap();
            assert_eq!(value.to_json(), text);
            assert!(json::stringify(&value, Some("\t")).is_some());
            let copy = value.clone();
            assert!(copy == value);
            drop(copy);
            drop(value);
        }
    });
}

#[test]
fn build_infos_read_3000_levels_of_metadata() {
    on_small_stack(|| {
        for nested in [arrays(3000), objects(3000)] {
            let text = format!(
                "{{ \"version\": 1, \"flameworkVersion\": \"x\", \"identifiers\": {{}}, \"metadata\": {{ \"other\": {nested} }} }}"
            );
            assert_eq!(find_problem(Some(&text)), None);
            let info = BuildInfo::from_value("x", json::parse(&text).unwrap());
            assert!(info.to_json().contains("\"other\""));
        }
    });
}

#[test]
fn configs_nested_too_deep_are_refused() {
    on_small_stack(|| {
        let deep = |n: usize| {
            format!(
                "{{ \"transformer\": {{ \"plugins\": [{{ \"path\": \"a\", \"options\": {} }}] }} }}",
                objects(n)
            )
        };
        // 100 levels read; 1,000 are refused, as TypeScript refuses them.
        assert!(jsonc::parse_config_text(&deep(100)).is_ok());
        for text in [
            deep(1000),
            deep(100_000),
            format!("{{ \"x\": {} }}", arrays(100_000)),
        ] {
            let error = jsonc::parse_config_text(&text).unwrap_err();
            assert!(
                error.starts_with("Maximum call stack size exceeded"),
                "{error}"
            );
        }

        let dir = std::env::temp_dir().join(format!("fwrs-deep-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("root")).unwrap();
        let file = dir.join("root").join("flamework.config.json");
        std::fs::write(&file, deep(jsonc::MAX_DEPTH - 6)).unwrap();
        let path = file.to_string_lossy().into_owned();
        assert!(config::read_project_config(&path, &Env::new()).is_ok());
        std::fs::write(&file, deep(1000)).unwrap();
        let error = config::read_project_config(&path, &Env::new()).unwrap_err();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            error.0.starts_with(&format!(
                "Failed to parse {path}: Maximum call stack size exceeded"
            )),
            "{error}"
        );
    });
}
