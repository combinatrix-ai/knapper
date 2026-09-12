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
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let expected = if args.first() == Some(&"lint") {
            i32::from(report["summary"]["total_issues"].as_u64().unwrap() > 0)
        } else {
            0
        };
        assert_eq!(
            output.status.code(),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        report
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
fn path_rules_use_unanchored_matching_and_last_matching_check_wins() {
    let vault = Vault::new(
        "lint:\n  rules:\n    broken-links:\n      enabled: false\n  paths:\n    - path: Questions\n      broken-links: true\n    - path: ^Questions/\n      broken-links:\n        pattern: ^legacy/\n",
        &[
            (
                "Questions/one.md",
                "Links [[legacy/Missing]] and [[other/Missing]].\n",
            ),
            ("Archive/Questions/two.md", "Link [[outside/Missing]].\n"),
        ],
    );

    let result = vault.json(&["lint", "--check", "broken-links", "--format", "json"]);
    assert_eq!(result["summary"]["broken_links"], 2);
    let details: Vec<_> = result["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| issue["detail"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(details, ["[[outside/Missing]]", "[[legacy/Missing]]"]);
}

#[test]
fn a_path_rule_can_enable_a_globally_disabled_check_by_default() {
    let vault = Vault::new(
        concat!(
            "lint:\n",
            "  rules:\n",
            "    broken-links:\n      enabled: false\n",
            "    orphans:\n      enabled: false\n",
            "    duplicates:\n      enabled: false\n",
            "    empty:\n      enabled: false\n",
            "    frontmatter:\n      enabled: false\n",
            "  paths:\n    - path: ^Projects/\n      broken-links: true\n",
        ),
        &[("Projects/source.md", "A link to [[Missing]].\n")],
    );

    let result = vault.json(&["lint", "--format", "json"]);
    assert_eq!(result["summary"]["broken_links"], 1);
    assert!(result["summary"].get("orphans").is_none());
}

#[test]
fn path_boolean_shorthand_filters_orphans() {
    let vault = Vault::new(
        "lint:\n  paths:\n    - path: ^Notes/\n      orphans: false\n",
        &[
            (
                "Notes/ignored.md",
                "This note is intentionally out of scope.\n",
            ),
            ("Other/reported.md", "This note remains an orphan.\n"),
        ],
    );

    let result = vault.json(&["lint", "--check", "orphans", "--format", "json"]);
    assert_eq!(result["summary"]["orphans"], 1);
    assert_eq!(result["issues"][0]["file"], "Other/reported.md");
}

#[test]
fn path_frontmatter_policy_checks_fields_and_counts_them_in_total() {
    let vault = Vault::new(
        concat!(
            "lint:\n",
            "  rules:\n",
            "    broken-links:\n      enabled: false\n",
            "    orphans:\n      enabled: false\n",
            "    duplicates:\n      enabled: false\n",
            "    empty:\n      enabled: false\n",
            "    frontmatter:\n      enabled: false\n",
            "  paths:\n",
            "    - path: ^Questions/\n",
            "      frontmatter:\n",
            "        required: [status]\n",
            "        fields:\n",
            "          status:\n            type: string\n            enum: [open, done]\n",
            "          priority:\n            type: number\n",
            "          tags:\n            type: list\n",
            "          options:\n            type: object\n",
            "          due:\n            type: date\n",
            "          published:\n            type: boolean\n",
            "          kind:\n            type: string\n",
            "            required_if:\n              field: status\n              equals: open\n",
        ),
        &[
            (
                "Questions/ok.md",
                "---\nstatus: open\npriority: 2\ntags: [one]\noptions: {mode: fast}\ndue: 2026-09-04\npublished: true\nkind: decision\n---\nA sufficiently long body.\n",
            ),
            (
                "Questions/bad.md",
                "---\nstatus: 3\npriority: high\ntags: one\noptions: [fast]\ndue: 2026-9-4\npublished: \"yes\"\n---\nA sufficiently long body.\n",
            ),
            (
                "Questions/conditional.md",
                "---\nstatus: open\n---\nA sufficiently long body.\n",
            ),
            ("Questions/missing.md", "A sufficiently long body without metadata.\n"),
        ],
    );

    let result = vault.json(&["lint", "--check", "frontmatter", "--format", "json"]);
    assert_eq!(result["summary"]["missing_frontmatter"], 1);
    assert_eq!(result["summary"]["frontmatter_errors"], 8);
    assert_eq!(result["summary"]["total_issues"], 9);

    let policy_issues: Vec<_> = result["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|issue| issue.get("field").is_some())
        .collect();
    assert_eq!(policy_issues.len(), 8);
    assert!(policy_issues.iter().all(|issue| {
        issue["severity"] == "info"
            && issue["file"].is_string()
            && issue["field"].is_string()
            && issue["detail"].is_string()
    }));
    assert!(policy_issues
        .iter()
        .any(|issue| { issue["file"] == "Questions/conditional.md" && issue["field"] == "kind" }));
    assert!(policy_issues
        .iter()
        .any(|issue| { issue["file"] == "Questions/bad.md" && issue["field"] == "due" }));
}

#[test]
fn path_frontmatter_false_suppresses_policy_and_missing_checks() {
    let vault = Vault::new(
        concat!(
            "lint:\n",
            "  rules:\n",
            "    broken-links:\n      enabled: false\n",
            "    orphans:\n      enabled: false\n",
            "    duplicates:\n      enabled: false\n",
            "    empty:\n      enabled: false\n",
            "    frontmatter:\n      enabled: false\n",
            "  paths:\n",
            "    - path: ^Questions/\n",
            "      frontmatter:\n        required: [status]\n",
            "    - path: ^Questions/private/\n      frontmatter: false\n",
        ),
        &[
            ("Questions/public.md", "Public note without metadata.\n"),
            (
                "Questions/private/secret.md",
                "Private note without metadata.\n",
            ),
        ],
    );

    let result = vault.json(&["lint", "--check", "frontmatter", "--format", "json"]);
    assert_eq!(result["summary"]["missing_frontmatter"], 1);
    assert_eq!(result["summary"]["frontmatter_errors"], 1);
    assert_eq!(result["summary"]["total_issues"], 2);
    assert!(!result["issues"].to_string().contains("secret.md"));
}

#[test]
fn null_frontmatter_values_are_unset_for_policy_checks() {
    let vault = Vault::new(
        concat!(
            "lint:\n  paths:\n    - path: ^Questions/\n      frontmatter:\n",
            "        required: [status]\n",
            "        fields:\n",
            "          decided:\n            type: date\n",
            "            required_if:\n              field: status\n              equals: decided\n",
        ),
        &[
            (
                "Questions/open.md",
                "---\nstatus: open\ndecided:\n---\nA sufficiently long body.\n",
            ),
            (
                "Questions/decided.md",
                "---\nstatus: decided\ndecided:\n---\nA sufficiently long body.\n",
            ),
            (
                "Questions/status-null.md",
                "---\nstatus:\n---\nA sufficiently long body.\n",
            ),
        ],
    );

    let result = vault.json(&["lint", "--check", "frontmatter", "--format", "json"]);
    assert_eq!(result["summary"]["frontmatter_errors"], 2);
    assert_eq!(result["summary"]["total_issues"], 2);
    assert!(!result["issues"].to_string().contains("open.md"));
    assert!(result["issues"].to_string().contains("decided.md"));
    assert!(result["issues"].to_string().contains("status-null.md"));
}

#[test]
fn unmatched_path_rules_leave_global_rule_behavior_unchanged() {
    let vault = Vault::new(
        concat!(
            "lint:\n",
            "  rules:\n",
            "    broken-links:\n      enabled: false\n",
            "    orphans:\n      enabled: false\n",
            "    duplicates:\n      enabled: false\n",
            "    empty:\n      enabled: false\n",
            "    frontmatter:\n      enabled: true\n",
            "  paths:\n    - path: ^Questions/\n      frontmatter: false\n",
        ),
        &[
            ("Questions/ignored.md", "Question without metadata.\n"),
            ("Notes/global.md", "Global rule still applies here.\n"),
        ],
    );

    let result = vault.json(&["lint", "--check", "frontmatter", "--format", "json"]);
    assert_eq!(result["summary"]["missing_frontmatter"], 1);
    assert_eq!(result["summary"]["total_issues"], 1);
    assert_eq!(result["issues"][0]["file"], "Notes/global.md");
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

#[test]
fn same_note_wikilink_anchors_are_not_broken_or_graph_edges() {
    let vault = Vault::new(
        "",
        &[(
            "Note.md",
            "# Note\n\nSee [[#Section]], [[#Section|below]], and [[^block-id]].\n\n## Section\n\nA paragraph. ^block-id\n",
        )],
    );

    let broken = vault.json(&["broken-links", "--format", "json"]);
    assert_eq!(broken, serde_json::json!([]));

    let lint = vault.json(&["lint", "--check", "broken-links", "--format", "json"]);
    assert_eq!(lint["summary"]["broken_links"], 0);
    assert_eq!(lint["summary"]["total_issues"], 0);

    let orphans = vault.json(&["orphans", "--format", "json"]);
    assert_eq!(orphans, serde_json::json!(["Note.md"]));
}

#[test]
fn missing_headings_and_blocks_are_broken_with_positions() {
    let vault = Vault::new(
        "",
        &[
            (
                "Source.md",
                "# Source\n\n[[#Missing Local]]\n[[Target#Missing Remote]]\n[[Target#^missing-block]]\n[local](#missing-markdown)\n",
            ),
            ("Target.md", "# Target\n\n## Existing\n\nText. ^existing-block\n"),
        ],
    );

    let broken = vault.json(&["broken-links", "--format", "json"]);
    assert_eq!(broken.as_array().unwrap().len(), 4, "{broken:#}");
    assert_eq!(broken[0]["target"], "#Missing Local");
    assert_eq!(broken[0]["reason"], "missing-heading");
    assert_eq!(broken[1]["target"], "Target#Missing Remote");
    assert_eq!(broken[1]["reason"], "missing-heading");
    assert_eq!(broken[2]["target"], "Target#^missing-block");
    assert_eq!(broken[2]["reason"], "missing-block");
    assert_eq!(broken[3]["target"], "#missing-markdown");
    assert_eq!(broken[3]["syntax"], "markdown");

    let lint = vault.json(&["lint", "--check", "broken-links", "--format", "json"]);
    assert_eq!(
        lint["summary"]["broken_links"],
        broken.as_array().unwrap().len()
    );

    let query = vault.json(&["query", "--where", "broken>0", "--format", "json"]);
    assert_eq!(query.as_array().unwrap().len(), 1, "{query:#}");
    assert_eq!(query[0]["path"], "Source.md");
    assert_eq!(query[0]["broken"], 4);
}

#[test]
fn valid_cross_note_anchors_are_edges_and_anchor_syntax_is_normalized() {
    let vault = Vault::new(
        "",
        &[
            (
                "Source.md",
                "[[Target#Mixed Case]]\n[[Target#Mixed Case ###]]\n[[Target#^BLOCK_id]]\n[heading](Target.md#Mixed%20Case)\n[block](Target.md#^block_id)\n",
            ),
            (
                "Target.md",
                "## Mixed Case ###\n\nParagraph. ^block_id\n",
            ),
        ],
    );

    assert_eq!(
        vault.json(&["broken-links", "--format", "json"]),
        serde_json::json!([])
    );
    let orphans = vault.json(&["orphans", "--format", "json"]);
    assert_eq!(orphans, serde_json::json!(["Source.md"]));
}

#[test]
fn required_headings_use_prose_anchors_and_report_missing_titles() {
    let vault = Vault::new(
        "lint:\n  paths:\n    - path: '^People/'\n      headings:\n        required: [Bio, 出典, 関係, bio]\n",
        &[
            ("People/Good.md", "---\ntype: person\n---\n## BIO ###\n出典\n---\n### 関係\n"),
            ("People/Bad.md", "---\nexample: |\n  ## Bio\n---\n```md\n## Bio\n```\n%%\n## 出典\n%%\n関係 is prose\n"),
            ("Other.md", "No headings\n"),
        ],
    );
    let report = vault.json(&["lint", "--check", "headings", "--format", "json"]);
    assert_eq!(report["summary"]["missing_headings"], 3);
    assert_eq!(report["summary"]["total_issues"], 3);
    let issues = report["issues"].as_array().unwrap();
    assert!(issues
        .iter()
        .all(|i| i["file"] == "People/Bad.md" && i["type"] == "headings"));
    let text = vault.run(&["lint", "--check", "headings"]);
    assert!(String::from_utf8_lossy(&text.stdout)
        .contains("People/Bad.md: Missing required heading: Bio"));
}

#[test]
fn heading_policies_obey_path_overrides_exclusions_and_org_blocks() {
    let vault = Vault::new(
        "exclude: [Excluded]\nlint:\n  rules:\n    headings:\n      enabled: false\n      exclude: [People/Ignored.md]\n  paths:\n    - path: '^People/'\n      headings:\n        required: [Bio]\n    - path: '^People/Skip'\n      headings: false\n    - path: '^People/Override'\n      headings:\n        required: [出典]\n    - path: '^Excluded/'\n      headings:\n        required: [Bio]\n",
        &[
            ("People/Good.org", "* Bio\n"),
            ("People/Bad.org", "#+BEGIN_SRC text\n* Bio\n#+END_SRC\n"),
            ("People/Skip.md", "none"),
            ("People/Override.md", "## 出典\n"),
            ("People/Ignored.md", "none"),
            ("Excluded/X.md", "none"),
        ],
    );
    for args in [
        vec!["lint", "--format", "json"],
        vec!["lint", "--check", "headings", "--format", "json"],
    ] {
        let report = vault.json(&args);
        assert_eq!(report["summary"]["missing_headings"], 2);
        assert!(report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["file"] == "People/Bad.org" && i["type"] == "headings"));
    }
}

#[test]
fn heading_config_is_strict_and_schema_exposes_it() {
    for settings in [
        "required: Bio",
        "required: [42]",
        "required: ['  ']",
        "enabled: nope",
        "requried: [Bio]",
    ] {
        let vault = Vault::new(
            &format!("lint:\n  paths:\n    - path: People\n      headings:\n        {settings}\n"),
            &[],
        );
        assert!(
            !vault.run(&["config", "check"]).status.success(),
            "{settings}"
        );
    }
    let vault = Vault::new(
        "lint:\n  paths:\n    - path: People\n      headings: true\n",
        &[("People/A.md", "nothing")],
    );
    assert_eq!(
        vault.json(&["lint", "--check", "headings", "--format", "json"])["summary"]
            ["missing_headings"],
        0
    );
    let schema = vault.json(&["config", "schema"]);
    assert!(schema["$defs"]["lintPathHeadings"].is_object());
}

#[test]
fn lint_exit_status_tracks_findings_in_text_and_json() {
    let vault = Vault::new(
        "lint:\n  paths:\n    - path: '^People/'\n      headings:\n        required: [Bio]\n",
        &[("People/A.md", "## Other\n")],
    );
    for format in ["text", "json"] {
        let output = vault.run(&["lint", "--check", "headings", "--format", format]);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.is_empty());
        assert!(String::from_utf8_lossy(&output.stdout).contains("Bio"));
    }
    fs::write(vault.root.path().join("People/A.md"), "## Bio\n").unwrap();
    for format in ["text", "json"] {
        let output = vault.run(&["lint", "--check", "headings", "--format", format]);
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn markdown_heading_slugs_resolve_without_weakening_required_titles() {
    let vault = Vault::new(
        "lint:\n  paths:\n    - path: Note\n      headings:\n        required: [quick-start]\n",
        &[("Note.md", "## Quick Start\n## Quick Start\n## What's Next?\n## 日本語 見出し\n\n[one](#quick-start) [two](#quick-start-1) [next](#whats-next) [jp](#日本語-見出し) [missing](#quick-start-2)\n")],
    );
    let broken = vault.json(&["broken-links", "--format", "json"]);
    assert_eq!(broken.as_array().unwrap().len(), 1);
    assert_eq!(broken[0]["target"], "#quick-start-2");
    let report = vault.json(&["lint", "--check", "headings", "--format", "json"]);
    assert_eq!(report["summary"]["missing_headings"], 1);
}

#[test]
fn rule_filters_share_query_fields_and_combine_with_paths() {
    let vault = Vault::new(
        "lint:\n  rules:\n    headings: {enabled: false}\n  paths:\n    - where: [type=manifest, '!archived']\n      headings: {required: [Result]}\n    - path: '^Notes/'\n      where: [status=done, words>2]\n      headings: {required: [Summary]}\n",
        &[
            ("Notes/a.md", "---\ntype: manifest\nstatus: done\n---\nEnough words in this note.\n"),
            ("Other/b.md", "---\ntype: manifest\nstatus: done\n---\nEnough words in this note.\n"),
            ("Other/skip.md", "---\ntype: manifest\narchived: true\n---\nNo heading\n"),
            ("Other/inline.md", "type:: manifest\nEnough words in this note.\n"),
            ("Other/missing-type.md", "No metadata\n"),
        ],
    );
    let report = vault.json(&["lint", "--check", "headings", "--format", "json"]);
    let issues = report["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 3, "{report}");
    assert!(issues
        .iter()
        .any(|i| i["file"] == "Notes/a.md" && i["heading"] == "Summary"));
    assert!(issues
        .iter()
        .any(|i| i["file"] == "Other/b.md" && i["heading"] == "Result"));
    assert!(issues.iter().any(|i| i["file"] == "Other/inline.md"));
}

#[test]
fn rule_filters_can_use_graph_fields_without_losing_unselected_inlinks() {
    let vault = Vault::new(
        "lint:\n  paths:\n    - where: [inlinks>0]\n      headings: {required: [Summary]}\n",
        &[
            ("Selected.md", "No heading\n"),
            ("Other.md", "[[Selected]]\n"),
        ],
    );
    let report = vault.json(&[
        "lint",
        "Selected.md",
        "--check",
        "headings",
        "--format",
        "json",
    ]);
    assert_eq!(report["summary"]["missing_headings"], 1);
}

#[test]
fn invalid_filter_config_is_rejected() {
    for selector in [
        "where: type=manifest",
        "where: []",
        "where: [4]",
        "where: ['']",
        "where: ['!']",
    ] {
        let vault = Vault::new(
            &format!("lint:\n  paths:\n    - {selector}\n      headings: true\n"),
            &[],
        );
        let out = vault.run(&["config", "check"]);
        assert!(!out.status.success(), "{selector}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("where"));
    }
}

#[test]
fn explicit_files_limit_reports_but_not_reference_or_duplicate_context() {
    let vault = Vault::new(
        "",
        &[
            ("Chosen/A.md", "---\ntype: note\n---\n[[Target]]\n"),
            ("Other/A.md", "A duplicate with enough content.\n"),
            ("Target.md", "[[Chosen/A]] and [[Missing]]\n"),
        ],
    );
    let report = vault.json(&[
        "lint",
        "Chosen/A.md",
        "--check",
        "broken-links",
        "--check",
        "orphans",
        "--check",
        "duplicates",
        "--format",
        "json",
    ]);
    assert_eq!(report["scope"]["files"], serde_json::json!(["Chosen/A.md"]));
    assert_eq!(report["summary"]["broken_links"], 0);
    assert_eq!(report["summary"]["orphans"], 0);
    assert_eq!(report["summary"]["duplicates"], 1);
    assert_eq!(report["issues"][0]["file"], "Chosen/A.md");
    assert!(report["issues"][0]["detail"]
        .as_str()
        .unwrap()
        .contains("Other/A.md"));
    assert!(!vault.run(&["lint", "does-not-exist.md"]).status.success());
}

#[test]
fn explicit_files_support_spaces_multiple_absolute_paths_and_exclusions() {
    let vault = Vault::new(
        "exclude: [Excluded]\n",
        &[
            ("Space Name.md", "x"),
            ("日本語.md", "x"),
            ("Other.md", "x"),
            ("Excluded/A.md", "x"),
        ],
    );
    let absolute = vault.root.path().join("日本語.md");
    let report = vault.json(&[
        "lint",
        "Space Name.md",
        absolute.to_str().unwrap(),
        "--check",
        "empty",
        "--format",
        "json",
    ]);
    assert_eq!(report["summary"]["empty"], 2);
    assert!(!vault.run(&["lint", "Excluded/A.md"]).status.success());
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("A.md"), "x").unwrap();
    assert!(!vault
        .run(&["lint", outside.path().join("A.md").to_str().unwrap()])
        .status
        .success());
}

fn git(vault: &Vault, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(vault.root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_git(vault: &Vault) {
    git(vault, &["init", "-q"]);
    git(vault, &["config", "user.name", "Test"]);
    git(vault, &["config", "user.email", "test@example.invalid"]);
}

#[test]
fn git_diff_includes_staged_unstaged_untracked_and_renames_but_not_deleted_or_ignored() {
    let vault = Vault::new(
        "",
        &[
            ("staged.md", "long enough initial content"),
            ("unstaged.md", "long enough initial content"),
            ("old.md", "long enough initial content"),
            ("deleted.md", "long enough initial content"),
            ("unchanged.md", "x"),
        ],
    );
    init_git(&vault);
    git(&vault, &["add", "."]);
    git(&vault, &["commit", "-qm", "initial"]);
    fs::write(vault.root.path().join("staged.md"), "x").unwrap();
    git(&vault, &["add", "staged.md"]);
    fs::write(vault.root.path().join("unstaged.md"), "x").unwrap();
    fs::write(vault.root.path().join("untracked 日本語\n.md"), "x").unwrap();
    git(&vault, &["mv", "old.md", "renamed.md"]);
    git(&vault, &["rm", "deleted.md"]);
    fs::write(vault.root.path().join(".gitignore"), "ignored.md\n").unwrap();
    fs::write(vault.root.path().join("ignored.md"), "x").unwrap();
    let report = vault.json(&["lint", "--diff", "--check", "empty", "--format", "json"]);
    assert_eq!(
        report["scope"]["files"],
        serde_json::json!([
            "renamed.md",
            "staged.md",
            "unstaged.md",
            "untracked 日本語\n.md"
        ])
    );
    assert_eq!(report["summary"]["empty"], 3);
    assert!(!vault
        .run(&["lint", "--diff", "no-such-ref"])
        .status
        .success());
    assert_eq!(
        vault
            .run(&["lint", "--diff", "HEAD", "staged.md"])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn git_diff_supports_unborn_repositories_and_empty_selection() {
    let vault = Vault::new("", &[("A.md", "x")]);
    assert!(!vault.run(&["lint", "--diff"]).status.success());
    init_git(&vault);
    let report = vault.json(&["lint", "--diff", "--check", "empty", "--format", "json"]);
    assert_eq!(report["summary"]["empty"], 1);
    git(&vault, &["add", "."]);
    git(&vault, &["commit", "-qm", "initial"]);
    let report = vault.json(&["lint", "--diff", "--format", "json"]);
    assert_eq!(report["scope"]["files"], serde_json::json!([]));
    assert_eq!(report["summary"]["total_issues"], 0);
}

#[test]
fn git_diff_reference_and_nested_vault_use_repo_relative_git_paths() {
    let vault = Vault::new(
        "",
        &[
            ("Nested/A.md", "long enough initial content"),
            ("Outside.md", "x"),
        ],
    );
    init_git(&vault);
    git(&vault, &["add", "."]);
    git(&vault, &["commit", "-qm", "initial"]);
    git(&vault, &["tag", "base"]);
    fs::write(vault.root.path().join("Nested/A.md"), "x").unwrap();
    git(&vault, &["add", "."]);
    git(&vault, &["commit", "-qm", "change"]);
    let report = vault.json(&[
        "lint", "--diff", "base", "--check", "empty", "--format", "json",
    ]);
    assert_eq!(report["scope"]["files"], serde_json::json!(["Nested/A.md"]));
    fs::write(vault.root.path().join("Nested/A.md"), "z").unwrap();
    fs::write(vault.root.path().join("Outside.md"), "z").unwrap();
    fs::write(vault.root.path().join("Nested/new.md"), "x").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_knapper"))
        .args([
            "--vault",
            vault.root.path().join("Nested").to_str().unwrap(),
            "lint",
            "--diff",
            "--check",
            "empty",
            "--format",
            "json",
        ])
        .current_dir(vault.root.path())
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        report["scope"]["files"],
        serde_json::json!(["A.md", "new.md"])
    );
}

#[test]
fn existing_directory_links_resolve_without_readme_or_note_graph_nodes() {
    let vault = Vault::new("", &[("Notes/Source.md", "[empty](../Empty/) [folder](../Docs/) [local](./) [parent](../) [[Docs/]] [missing](../Absent/)\n"), ("Docs/Child.md", "A body with enough content.\n"), ("Other/Absent.md", "x")]);
    fs::create_dir(vault.root.path().join("Empty")).unwrap();
    let report = vault.json(&[
        "lint",
        "Notes/Source.md",
        "--check",
        "broken-links",
        "--format",
        "json",
    ]);
    assert_eq!(report["summary"]["broken_links"], 1, "{report}");
    let broken = vault.json(&["broken-links", "--format", "json"]);
    assert_eq!(broken.as_array().unwrap().len(), 1);
    let query = vault.json(&[
        "query",
        "--where",
        "path=Notes/Source.md",
        "--field",
        "outlinks",
        "--format",
        "json",
    ]);
    assert_eq!(query[0]["outlinks"], 0);
}

#[cfg(unix)]
#[test]
fn directory_links_do_not_escape_through_paths_or_symlinks() {
    let vault = Vault::new(
        "",
        &[(
            "A.md",
            "[escape](../) [symlink](Outside/) [hidden](.hidden/)\n",
        )],
    );
    let outside = tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), vault.root.path().join("Outside")).unwrap();
    fs::create_dir(vault.root.path().join(".hidden")).unwrap();
    let report = vault.json(&["lint", "--check", "broken-links", "--format", "json"]);
    assert_eq!(report["summary"]["broken_links"], 3);
}

#[test]
fn file_local_lint_does_not_read_unselected_note_bodies_for_filters() {
    let vault = Vault::new(
        "lint:\n  paths:\n    - where: [type=manifest]\n      headings: {required: [Result]}\n",
        &[("A.md", "---\ntype: manifest\n---\n## Result\n")],
    );
    fs::write(vault.root.path().join("Unreadable.md"), [0xff, 0xfe]).unwrap();
    let report = vault.json(&["lint", "A.md", "--check", "headings", "--format", "json"]);
    assert_eq!(report["summary"]["total_issues"], 0);
}
