use serde_json::Value;
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;
fn fixture() -> TempDir {
    let v = TempDir::new().unwrap();
    fs::write(
        v.path().join("knapper.yaml"),
        "vault_path: .\nexclude: [Excluded]\n",
    )
    .unwrap();
    fs::create_dir(v.path().join("Excluded")).unwrap();
    v
}
fn run(v: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_knapper"))
        .env("KNAPPER_NO_UPDATE_CHECK", "1")
        .current_dir(v.path())
        .args(args)
        .output()
        .unwrap()
}
fn success(out: Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn linter_actual_app_references_preview_apply_and_idempotence() {
    let v = fixture();
    let cases: Value =
        serde_json::from_str(include_str!("fixtures/pkm/linter-cases.json")).unwrap();
    for c in cases["cases"].as_array().unwrap() {
        let before = c["before"].as_str().unwrap();
        fs::write(v.path().join("Input.md"), before).unwrap();
        let mut args = vec!["format-note", "Input.md", "--format", "json"];
        for r in c["rules"].as_array().unwrap() {
            args.extend(["--rule", r.as_str().unwrap()]);
        }
        let preview = success(run(&v, &args));
        assert_eq!(preview["after"], c["expected"], "{}", c["id"]);
        assert_eq!(
            fs::read_to_string(v.path().join("Input.md")).unwrap(),
            before
        );
        args.push("--apply");
        let applied = success(run(&v, &args));
        assert_eq!(applied["after"], c["expected"]);
        assert_eq!(
            fs::read_to_string(v.path().join("Input.md")).unwrap(),
            c["expected"].as_str().unwrap()
        );
        assert_eq!(
            success(run(&v, &args))["after"],
            c["expectedSecondPass"],
            "{}",
            c["id"]
        );
    }
}
#[test]
fn quickadd_actual_app_references_preview_and_apply() {
    let v = fixture();
    let cases: Value =
        serde_json::from_str(include_str!("fixtures/pkm/quickadd-cases.json")).unwrap();
    for c in cases["cases"].as_array().unwrap() {
        let before = c["before"].as_str().unwrap();
        fs::write(v.path().join("Log.md"), before).unwrap();
        fs::write(
            v.path().join("Template.md"),
            c["template"].as_str().unwrap(),
        )
        .unwrap();
        let mut args = vec![
            "capture",
            "Log.md",
            "--text",
            c["text"].as_str().unwrap(),
            "--template",
            "Template.md",
            "--position",
            c["position"].as_str().unwrap(),
            "--format",
            "json",
        ];
        let preview = success(run(&v, &args));
        assert_eq!(preview["after"], c["expected"], "{}", c["id"]);
        assert_eq!(fs::read_to_string(v.path().join("Log.md")).unwrap(), before);
        args.push("--apply");
        success(run(&v, &args));
        assert_eq!(
            fs::read_to_string(v.path().join("Log.md")).unwrap(),
            c["expected"].as_str().unwrap()
        );
    }
}
#[test]
fn plugin_operations_reject_outside_excluded_and_unsupported_templates() {
    let v = fixture();
    let other = TempDir::new().unwrap();
    let outside = other.path().join("Outside.md");
    fs::write(&outside, "private").unwrap();
    fs::write(v.path().join("Excluded/Input.md"), "excluded").unwrap();
    fs::write(v.path().join("Log.md"), "old").unwrap();
    fs::write(v.path().join("Template.md"), "{{DATE}} <% js %>").unwrap();
    for args in [
        vec![
            "format-note",
            outside.to_str().unwrap(),
            "--rule",
            "trailing-spaces",
            "--apply",
        ],
        vec!["capture", "Excluded/Input.md", "--text", "new", "--apply"],
        vec![
            "capture",
            "Excluded/New.md",
            "--text",
            "new",
            "--create",
            "--apply",
        ],
        vec![
            "capture",
            "Log.md",
            "--text",
            "new",
            "--template",
            "Template.md",
            "--apply",
        ],
    ] {
        assert!(!run(&v, &args).status.success());
    }
    assert_eq!(fs::read_to_string(outside).unwrap(), "private");
    assert_eq!(fs::read_to_string(v.path().join("Log.md")).unwrap(), "old");
    assert!(!v.path().join("Excluded/New.md").exists());
}
#[test]
fn capture_creation_requires_flag_and_preview_does_not_create() {
    let v = fixture();
    assert!(!run(&v, &["capture", "New.md", "--text", "new", "--apply"])
        .status
        .success());
    success(run(
        &v,
        &[
            "capture", "New.md", "--text", "new", "--create", "--format", "json",
        ],
    ));
    assert!(!v.path().join("New.md").exists());
    success(run(
        &v,
        &[
            "capture", "New.md", "--text", "new", "--create", "--apply", "--format", "json",
        ],
    ));
    assert_eq!(fs::read_to_string(v.path().join("New.md")).unwrap(), "new");
}
#[cfg(unix)]
#[test]
fn plugin_mutations_refuse_symlink_escape() {
    let v = fixture();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("Secret.md"), "private").unwrap();
    std::os::unix::fs::symlink(outside.path(), v.path().join("Escape")).unwrap();
    assert!(!run(
        &v,
        &["capture", "Escape/Secret.md", "--text", "new", "--apply"]
    )
    .status
    .success());
    assert!(!run(
        &v,
        &[
            "capture",
            "Escape/New.md",
            "--text",
            "new",
            "--create",
            "--apply"
        ]
    )
    .status
    .success());
    assert_eq!(
        fs::read_to_string(outside.path().join("Secret.md")).unwrap(),
        "private"
    );
}
