//! End-to-end coverage for configurable vault-health rules.

use std::fs;
use std::process::{Command, Output};

use tempfile::{tempdir, TempDir};

struct Vault {
    root: TempDir,
}

impl Vault {
    fn new(config: &str, files: &[(&str, &str)]) -> Self {
        let root = tempdir().unwrap();
        fs::write(
            root.path().join("knapper.yaml"),
            format!("vault_path: .\n{config}"),
        )
        .unwrap();
        for (relative, content) in files {
            let path = root.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        Self { root }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_knapper"))
            .args(args)
            .current_dir(self.root.path())
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn disabled_rules_are_omitted_but_explicit_check_overrides_disabled() {
    let vault = Vault::new(
        "lint:\n  rules:\n    broken-links:\n      enabled: false\n    empty:\n      enabled: false\n",
        &[
            ("Broken.md", "[[Missing]]\n"),
            ("Orphan.md", "This note has enough prose.\n"),
            ("Tiny.md", "x\n"),
        ],
    );

    let plain = vault.json(&["lint", "--format", "json"]);
    assert!(plain["summary"].get("broken_links").is_none());
    assert!(plain["summary"].get("empty").is_none());
    assert_eq!(plain["summary"]["orphans"], 3);

    let explicit = vault.json(&["lint", "--check", "broken-links", "--format", "json"]);
    assert_eq!(explicit["summary"]["broken_links"], 1);
    assert_eq!(explicit["summary"]["total_issues"], 1);
}

#[test]
fn include_and_exclude_scope_broken_links_by_source_path() {
    let vault = Vault::new(
        "lint:\n  rules:\n    broken-links:\n      include: [Projects/]\n      exclude: [Projects/private/]\n",
        &[
            ("Projects/keep.md", "[[Missing Keep]]\n"),
            ("Projects/private/noise.md", "[[Missing Noise]]\n"),
            ("Notes/outside.md", "[[Missing Outside]]\n"),
        ],
    );

    let result = vault.json(&["lint", "--check", "broken-links", "--format", "json"]);
    assert_eq!(result["summary"]["broken_links"], 1);
    assert_eq!(result["issues"][0]["file"], "Projects/keep.md");
}

#[test]
fn duplicate_groups_need_two_scoped_paths_and_report_only_scoped_paths() {
    let vault = Vault::new(
        "lint:\n  rules:\n    duplicates:\n      include: [Notes/]\n      exclude: [Notes/archive/]\n",
        &[
            ("Notes/One.md", "A sufficiently long note body.\n"),
            ("Notes/nested/One.md", "Another sufficiently long body.\n"),
            ("Notes/archive/One.md", "An archived duplicate body.\n"),
            ("Other/One.md", "An out of scope duplicate body.\n"),
            ("Notes/Single.md", "Only one scoped path.\n"),
            ("Other/Single.md", "Only one other path.\n"),
        ],
    );

    let result = vault.json(&["lint", "--check", "duplicates", "--format", "json"]);
    assert_eq!(result["summary"]["duplicates"], 1);
    let issue = &result["issues"][0];
    assert_eq!(issue["file"], "Notes/One.md");
    assert_eq!(issue["detail"], "Also at: Notes/nested/One.md");
    assert!(!issue.to_string().contains("archive"));
}

#[test]
fn unknown_cli_check_is_rejected_as_usage_error() {
    let root = tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_knapper"))
        .args(["lint", "--check", "not-a-rule"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid value"));
}
