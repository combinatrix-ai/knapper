use serde_json::Value;
use std::process::Command;
#[test]
fn tasks_plugin_actual_app_references() {
    let vault = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pkm");
    let manifest: Value =
        serde_json::from_str(include_str!("fixtures/pkm/tasks-plugin-cases.json")).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let out = Command::new(env!("CARGO_BIN_EXE_knapper"))
            .env("KNAPPER_NO_UPDATE_CHECK", "1")
            .arg("-v")
            .arg(&vault)
            .arg("-c")
            .arg(vault.join("knapper.yaml"))
            .args([
                "tasks",
                "--query",
                case["query"].as_str().unwrap(),
                "--format",
                "json",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value, case["expected"], "{}", case["id"]);
    }
}
#[test]
fn tasks_query_rejects_native_flags_and_mutations() {
    let vault = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pkm");
    for args in [
        vec!["tasks", "--query", "done", "--all"],
        vec!["tasks", "--query", "done", "done", "Finish draft"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_knapper"))
            .env("KNAPPER_NO_UPDATE_CHECK", "1")
            .arg("-v")
            .arg(&vault)
            .args(args)
            .output()
            .unwrap();
        assert!(!out.status.success());
    }
}

#[test]
fn tasks_numeric_collation_gap_is_explicit() {
    let vault = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pkm");
    let gap: Value =
        serde_json::from_str(include_str!("fixtures/pkm/tasks-plugin-known-gaps.json")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_knapper"))
        .env("KNAPPER_NO_UPDATE_CHECK", "1")
        .arg("-v")
        .arg(&vault)
        .arg("-c")
        .arg(vault.join("knapper.yaml"))
        .args([
            "tasks",
            "--query",
            gap["query"].as_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    let descriptions: Vec<_> = result["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["tasks"].as_array().unwrap())
        .map(|t| t["description"].clone())
        .collect();
    assert_eq!(serde_json::json!(descriptions), gap["knapper"]);
    assert_ne!(gap["knapper"], gap["obsidian"]);
}
