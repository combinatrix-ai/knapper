//! Soft topic references: `#tag` lookups, and `demote`.
//!
//! What a caller sees is pinned in `contract/cases.yaml`, where it belongs.
//! What is here is everything that has to be checked against the disk rather
//! than against stdout -- that a dry run wrote nothing at all, that a rewrite
//! moved exactly one span and left the bytes around it alone, that CRLF and a
//! missing trailing newline survive -- plus the failure paths, which are only
//! provable by what they did *not* do.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

struct Vault {
    root: TempDir,
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Vault {
    fn new(files: &[(&str, &str)]) -> Self {
        let vault = Vault {
            root: tempfile::tempdir().unwrap(),
        };
        vault.write("knapper.yaml", "vault_path: .\n");
        for (path, content) in files {
            vault.write(path, content);
        }
        vault
    }

    fn path(&self) -> PathBuf {
        self.root.path().join("vault")
    }

    fn at(&self, relative: &str) -> PathBuf {
        self.path().join(relative)
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.at(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.at(relative))
            .unwrap_or_else(|e| panic!("{relative} unreadable: {e}"))
    }

    /// Every note in the vault, as bytes, so "wrote nothing" can be proved
    /// rather than asserted about the one file a test happened to look at.
    fn snapshot(&self) -> Vec<(String, Vec<u8>)> {
        let mut found = Vec::new();
        let mut stack = vec![self.path()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    stack.push(entry.path());
                    continue;
                }
                let relative = entry
                    .path()
                    .strip_prefix(self.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                found.push((relative, fs::read(entry.path()).unwrap()));
            }
        }
        found.sort();
        found
    }

    fn run(&self, args: &[&str]) -> Run {
        let output = Command::new(env!("CARGO_BIN_EXE_knapper"))
            .args(args)
            .current_dir(self.path())
            .output()
            .expect("knapper runs");
        Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

impl Run {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout was not json ({e}):\n{}", self.stdout))
    }

    fn ok(self) -> Self {
        assert_eq!(self.code, 0, "exit {}\n{}", self.code, self.stderr);
        self
    }
}

/// The vault the whole feature exists for: `[[COO採用]]` written as a label,
/// never as a claim that a note called COO採用 exists.
fn labelled_vault() -> Vault {
    Vault::new(&[
        (
            "Daily/2026-07-01.md",
            "---\ntitle: 2026-07-01\n---\n\n- 面接の準備 [[COO採用]]\n- 別件 [[COO採用|採用]]\n",
        ),
        ("Daily/2026-07-02.md", "[[COO採用]] の続き。\n"),
    ])
}

// ------------------------------------------------------------- dry running --

/// The whole point of a dry run is that it is free. Comparing every byte in
/// the vault is the only assertion that actually says so.
#[test]
fn a_dry_run_writes_nothing_at_all() {
    let vault = labelled_vault();
    let before = vault.snapshot();

    let run = vault.run(&["demote", "COO採用", "--dry-run"]).ok();
    assert!(run.stdout.contains("DRY RUN"), "{}", run.stdout);
    assert!(
        run.stdout.contains("Nothing was written."),
        "{}",
        run.stdout
    );

    assert_eq!(vault.snapshot(), before, "a dry run touched the vault");
}

/// A dry run has to report the plan it would carry out, not a summary of it:
/// every edit, with the file and the line, is what a caller acts on.
#[test]
fn a_dry_run_reports_every_proposed_edit_with_its_file_and_line() {
    let vault = labelled_vault();
    let run = vault
        .run(&["demote", "COO採用", "--dry-run", "--format", "json"])
        .ok();
    let json = run.json();

    assert_eq!(json["dry_run"], true);
    assert_eq!(json["applied"], false);
    assert_eq!(json["links_updated"], 2);
    assert_eq!(json["edits"][0]["file"], "Daily/2026-07-01.md");
    assert_eq!(json["edits"][0]["changes"][0]["line"], 5);
    assert_eq!(json["edits"][0]["changes"][0]["before"], "[[COO採用]]");
    assert_eq!(json["edits"][0]["changes"][0]["after"], "#COO採用");
    assert_eq!(json["edits"][1]["file"], "Daily/2026-07-02.md");
    assert_eq!(json["edits"][1]["changes"][0]["line"], 1);

    // And the one it will not touch, with the reason it will not.
    assert_eq!(json["skipped"][0]["file"], "Daily/2026-07-01.md");
    assert_eq!(json["skipped"][0]["line"], 6);
    assert_eq!(json["skipped"][0]["reason"], "alias");
}

// -------------------------------------------------------------- rewriting --

/// A demote replaces one span. Everything around it -- indentation, the
/// punctuation the line ends with, the trailing newline or its absence -- is
/// the user's prose and is not this command's to normalise.
#[test]
fn only_the_span_changes() {
    let vault = Vault::new(&[
        ("a.md", "  - 面接の準備 [[COO採用]]。\n"),
        ("b.md", "no trailing newline [[COO採用]]"),
        ("c.md", "line one\r\n[[COO採用]] windows\r\nline three\r\n"),
    ]);

    vault.run(&["demote", "COO採用"]).ok();

    assert_eq!(vault.read("a.md"), "  - 面接の準備 #COO採用。\n");
    assert_eq!(vault.read("b.md"), "no trailing newline #COO採用");
    assert_eq!(
        vault.read("c.md"),
        "line one\r\n#COO採用 windows\r\nline three\r\n",
        "CRLF line endings did not survive"
    );
}

/// Only the exact form is eligible, and the report has to name the rest. A
/// demote that quietly did some of the work would leave a vault where the
/// links that changed and the ones that did not look identical.
#[test]
fn every_form_a_tag_cannot_hold_is_left_alone_and_reported() {
    let source = "\
- exact: [[Topic]]
- alias: [[Topic|the topic]]
- anchor: [[Topic#Heading]]
- block: [[Topic^b12]]
- embed: ![[Topic]]
- path: [[Folder/Topic]]
- adjacent: see[[Topic]]
- absorbing: [[Topic]]tail
- fenced:

```
[[Topic]]
```

- inline: `[[Topic]]`
- comment: %% [[Topic]] %%
";
    let vault = Vault::new(&[("note.md", source)]);
    let run = vault.run(&["demote", "Topic", "--format", "json"]).ok();
    let json = run.json();

    assert_eq!(json["links_updated"], 1);
    let reasons: Vec<&str> = json["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["reason"].as_str().unwrap())
        .collect();
    assert_eq!(
        reasons,
        [
            "alias",
            "anchor",
            "anchor",
            "embed",
            "path-qualified",
            "adjacent text",
            "adjacent text",
        ]
    );

    // Code and comments hold no references at all, so they are neither
    // rewritten nor reported -- there is nothing there to report.
    let after = vault.read("note.md");
    assert_eq!(after, source.replacen("[[Topic]]", "#Topic", 1));
}

/// Frontmatter is not prose. A wikilink in a YAML value is a real link, but
/// `#Topic` in one is a string rather than a tag, so it is reported instead.
#[test]
fn a_frontmatter_link_is_reported_rather_than_rewritten() {
    let vault = Vault::new(&[(
        "note.md",
        "---\nrelated: \"[[Topic]]\"\n---\n\nbody [[Topic]]\n",
    )]);

    let run = vault.run(&["demote", "Topic", "--format", "json"]).ok();
    assert_eq!(run.json()["links_updated"], 1);
    assert_eq!(run.json()["skipped"][0]["reason"], "frontmatter");
    assert_eq!(run.json()["skipped"][0]["line"], 2);
    assert_eq!(
        vault.read("note.md"),
        "---\nrelated: \"[[Topic]]\"\n---\n\nbody #Topic\n"
    );
}

/// `exclude` hides a subtree from every whole-vault command, and a command
/// that writes has to honour that at least as hard as one that reads.
#[test]
fn an_excluded_note_is_never_rewritten() {
    let vault = Vault::new(&[
        ("note.md", "[[Topic]]\n"),
        ("Archives/old.md", "[[Topic]]\n"),
    ]);
    vault.write("knapper.yaml", "vault_path: .\nexclude:\n  - Archives/\n");

    let run = vault.run(&["demote", "Topic", "--format", "json"]).ok();
    assert_eq!(run.json()["links_updated"], 1);
    assert_eq!(vault.read("Archives/old.md"), "[[Topic]]\n");
    assert_eq!(vault.read("note.md"), "#Topic\n");
}

// -------------------------------------------------------------- refusals --

/// Demoting a link to a note that exists is not a relabelling, it is a
/// deletion: the note loses a backlink and the reference stops pointing
/// anywhere. It has to be asked for, and until it is, nothing is written.
#[test]
fn a_target_that_names_a_real_note_is_refused_until_it_is_asked_for() {
    let vault = Vault::new(&[("Topic.md", "# Topic\n"), ("note.md", "[[Topic]]\n")]);
    let before = vault.snapshot();

    let run = vault.run(&["demote", "Topic"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stderr.contains("Topic.md"), "{}", run.stderr);
    assert!(
        run.stderr.contains("--allow-existing-note"),
        "the refusal does not say how to proceed: {}",
        run.stderr
    );
    assert_eq!(vault.snapshot(), before, "a refusal touched the vault");

    vault
        .run(&["demote", "Topic", "--allow-existing-note"])
        .ok();
    assert_eq!(vault.read("note.md"), "#Topic\n");
}

/// A target nothing links to is a typo far more often than it is a no-op, and
/// reporting "0 links updated" for it reads like success.
#[test]
fn a_target_with_no_wikilinks_is_an_error() {
    let vault = Vault::new(&[("note.md", "[[Topic]]\n")]);
    let run = vault.run(&["demote", "Topci"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("No [[Topci]]"), "{}", run.stderr);
}

/// A target that cannot be spelled as a tag is refused before anything is
/// read, and the refusal carries the flag that would let it through.
#[test]
fn a_target_that_cannot_be_a_tag_is_refused_with_a_way_forward() {
    let vault = Vault::new(&[("note.md", "[[Daily Tasks]]\n")]);
    let before = vault.snapshot();

    let run = vault.run(&["demote", "Daily Tasks"]);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("--tag"), "{}", run.stderr);
    assert_eq!(vault.snapshot(), before);

    vault
        .run(&["demote", "Daily Tasks", "--tag", "work/daily"])
        .ok();
    assert_eq!(vault.read("note.md"), "#work/daily\n");
}

/// knapper reads org and does not rewrite it. A link it cannot touch is
/// reported with the line it is on, not silently dropped from the plan.
#[test]
fn an_org_link_is_reported_and_left_alone() {
    let vault = Vault::new(&[
        (
            "note.org",
            "#+title: Log\n\n* Heading\nsee [[Topic]] here\n",
        ),
        ("note.md", "[[Topic]]\n"),
    ]);

    let run = vault.run(&["demote", "Topic", "--format", "json"]).ok();
    assert_eq!(run.json()["links_updated"], 1);
    assert_eq!(run.json()["skipped"][0]["file"], "note.org");
    assert_eq!(run.json()["skipped"][0]["line"], 4);
    assert_eq!(run.json()["skipped"][0]["reason"], "org-mode");
    assert!(vault.read("note.org").contains("[[Topic]]"));
}

/// A write that fails part-way has to put back what it already wrote, and say
/// so when it cannot. `b.md` is read-only, so the write to it fails after
/// `a.md` has already been rewritten.
#[cfg(unix)]
#[test]
fn a_failed_write_puts_back_what_it_already_wrote() {
    use std::os::unix::fs::PermissionsExt;

    let vault = Vault::new(&[("a.md", "[[Topic]]\n"), ("b.md", "[[Topic]]\n")]);
    let locked = vault.at("b.md");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o444)).unwrap();
    if fs::write(&locked, "probe").is_ok() {
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
        eprintln!("skipped: this user can write to a read-only file");
        return;
    }

    let run = vault.run(&["demote", "Topic"]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stderr.contains("b.md"), "{}", run.stderr);
    assert!(
        run.stderr.contains("Nothing was changed"),
        "the rollback was not reported: {}",
        run.stderr
    );
    assert_eq!(vault.read("a.md"), "[[Topic]]\n", "a.md was not put back");
    assert_eq!(vault.read("b.md"), "[[Topic]]\n");
}

// --------------------------------------------------------- the contract --

/// The whole motivation, end to end: a broken hard link becomes a soft label
/// that is still navigable, and stops being a finding.
#[test]
fn a_demoted_label_stops_being_broken_and_stays_navigable() {
    let vault = labelled_vault();

    assert_eq!(
        vault
            .run(&["broken-links", "--format", "paths"])
            .ok()
            .stdout
            .lines()
            .count(),
        2
    );

    vault.run(&["demote", "COO採用"]).ok();

    let backlinks = vault
        .run(&["backlinks", "#COO採用", "--format", "paths"])
        .ok();
    assert_eq!(
        backlinks.stdout,
        "Daily/2026-07-01.md\nDaily/2026-07-02.md\n"
    );

    // The alias form is still a hard link, so it is still broken -- which is
    // the honest answer, and is why it was reported rather than rewritten.
    let broken = vault.run(&["broken-links", "--format", "paths"]).ok();
    assert_eq!(broken.stdout, "Daily/2026-07-01.md\n");
}

/// A tag is not a note, and nothing about looking one up may make it one.
#[test]
fn a_tag_never_becomes_a_graph_node() {
    let vault = Vault::new(&[
        ("Hub.md", "# Hub\n\n#COO採用 と #work/hiring\n"),
        ("Other.md", "[[Hub]] を見る #COO採用\n"),
    ]);

    vault
        .run(&["backlinks", "#COO採用", "--format", "json"])
        .ok();
    vault.run(&["context", "#COO採用", "--format", "json"]).ok();

    for command in ["orphans", "hubs", "broken-links"] {
        let out = vault.run(&[command, "--format", "json"]).ok().stdout;
        assert!(
            !out.contains("COO採用"),
            "{command} reported a tag as a note: {out}"
        );
    }
    assert_eq!(
        vault.run(&["orphans", "--format", "paths"]).ok().stdout,
        "Other.md\n"
    );
}
