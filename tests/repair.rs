//! Planning repairs for links that are already broken.
//!
//! What a caller sees is pinned in `contract/cases.yaml`, where it belongs.
//! What is here is everything that has to be checked against the disk rather
//! than against stdout: that a read-only command really is one, that the plan
//! is the same plan twice running, that a proposed edit is complete enough to
//! carry out from the JSON alone -- and that nothing outside the real vault
//! can ever become a destination.

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
        Vault::configured("", files)
    }

    fn configured(config: &str, files: &[(&str, &str)]) -> Self {
        let vault = Vault {
            root: tempfile::tempdir().unwrap(),
        };
        vault.write("knapper.yaml", &format!("vault_path: .\n{config}"));
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

    /// Every file in the vault, as bytes, so "wrote nothing" can be proved
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

    fn plan(&self) -> serde_json::Value {
        self.run(&["repair-links", "--dry-run", "--format", "json"])
            .ok()
            .json()
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

/// A vault whose links went stale outside knapper: a folder was reorganised,
/// and the tails of the paths are still exactly right.
///
/// Note what is *not* here: a stale path to an ordinary note written without
/// its extension. The resolver already answers those by basename, so they
/// never become broken links and are none of this command's business.
fn stale_vault() -> Vault {
    Vault::configured(
        "exclude:\n  - Archive\n",
        &[
            (
                "Index.md",
                "# Index\n\n\
                 - [[legacy/notes/Foo.md]]\n\
                 - [Old plan](old/Archive/Old%20Plan.md)\n\
                 - [[legacy/assets/paper.pdf]]\n\
                 - [[Nowhere At All]]\n",
            ),
            ("notes/Foo.md", "# Foo\n"),
            ("Archive/Old Plan.md", "# Old Plan\n"),
            ("assets/paper.pdf", "%PDF-1.4\n"),
        ],
    )
}

fn occurrences(plan: &serde_json::Value) -> &Vec<serde_json::Value> {
    plan["occurrences"]
        .as_array()
        .expect("occurrences is a list")
}

// ------------------------------------------------------------- read-only --

/// V1 plans and nothing else. Comparing every byte in the vault is the only
/// assertion that actually says so.
#[test]
fn a_dry_run_writes_nothing_at_all() {
    let vault = stale_vault();
    let before = vault.snapshot();

    let run = vault.run(&["repair-links", "--dry-run"]).ok();
    assert!(run.stdout.contains("DRY RUN"), "{}", run.stdout);
    assert!(
        run.stdout.contains("Nothing was written."),
        "{}",
        run.stdout
    );

    assert_eq!(vault.snapshot(), before, "a dry run touched the vault");
}

/// Omitting the flag must not be readable as consent to write. It is refused
/// before the vault is even scanned, and refused loudly enough to act on.
#[test]
fn without_dry_run_the_command_refuses_and_writes_nothing() {
    let vault = stale_vault();
    let before = vault.snapshot();

    let run = vault.run(&["repair-links"]);
    assert_eq!(
        run.code, 2,
        "stdout: {}\nstderr: {}",
        run.stdout, run.stderr
    );
    assert!(run.stderr.contains("--dry-run"), "{}", run.stderr);
    assert!(
        run.stdout.is_empty(),
        "it printed a plan anyway: {}",
        run.stdout
    );

    assert_eq!(vault.snapshot(), before, "a refusal touched the vault");
}

/// A plan a caller cannot diff is a plan they have to re-read every time.
#[test]
fn the_plan_is_byte_identical_between_runs() {
    let vault = stale_vault();
    let first = vault
        .run(&["repair-links", "--dry-run", "--format", "json"])
        .ok();
    let second = vault
        .run(&["repair-links", "--dry-run", "--format", "json"])
        .ok();
    assert_eq!(first.stdout, second.stdout);

    // And ordered the way it says: by source, then line.
    let plan = first.json();
    let keys: Vec<(String, u64)> = occurrences(&plan)
        .iter()
        .map(|o| {
            (
                o["source"].as_str().unwrap().to_string(),
                o["line"].as_u64().unwrap(),
            )
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "occurrences are not in a defined order");
}

// ------------------------------------------------------------- the plan --

/// The point of publishing a byte span with the text before and after is that
/// an apply can be built on the plan rather than on a second scan. So this
/// carries one out from the JSON alone -- splice each span, deepest first --
/// and asks knapper whether anything is still broken.
#[test]
fn the_published_spans_are_enough_to_carry_the_repair_out() {
    let vault = stale_vault();
    let plan = vault.plan();

    let mut edits: Vec<(String, usize, usize, String, String)> = occurrences(&plan)
        .iter()
        .filter(|o| o["status"] == "safe")
        .map(|o| {
            let edit = &o["edit"];
            (
                o["source"].as_str().unwrap().to_string(),
                edit["byte_start"].as_u64().unwrap() as usize,
                edit["byte_end"].as_u64().unwrap() as usize,
                edit["before"].as_str().unwrap().to_string(),
                edit["after"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(edits.len(), 3, "expected three safe repairs: {plan:#}");

    // Splice from the end, so every earlier offset still stands.
    edits.sort_by_key(|(source, start, ..)| (source.clone(), std::cmp::Reverse(*start)));
    for (source, start, end, before, after) in &edits {
        let mut content = fs::read_to_string(vault.at(source)).unwrap();
        assert_eq!(&content[*start..*end], before, "the span moved");
        content.replace_range(*start..*end, after);
        fs::write(vault.at(source), content).unwrap();
    }

    // Only the link that never had a candidate is left.
    let after = vault.run(&["broken-links", "--format", "json"]).ok().json();
    let left: Vec<&str> = after
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["target"].as_str().unwrap())
        .collect();
    assert_eq!(left, ["Nowhere At All"]);
}

/// Every repair is checked by resolving what would be written, so a proposal
/// can never be published on the strength of a search a reader would not
/// reproduce.
#[test]
fn every_safe_repair_names_a_file_that_exists() {
    let vault = stale_vault();
    let plan = vault.plan();
    for occurrence in occurrences(&plan) {
        if occurrence["status"] != "safe" {
            assert!(occurrence["edit"].is_null(), "{occurrence:#}");
            continue;
        }
        let edit = &occurrence["edit"];
        let destination = edit["resolves_to"].as_str().unwrap();
        assert!(
            vault.at(destination).exists(),
            "{destination} does not exist: {occurrence:#}"
        );
        assert_eq!(
            occurrence["candidates"].as_array().unwrap().len(),
            1,
            "a safe repair with more than one destination: {occurrence:#}"
        );
    }
}

// ---------------------------------------------------------------- safety --

/// A symlinked directory can lead anywhere, and the vault walk does not
/// descend one. Nothing behind it may become a repair destination -- however
/// exactly its path happens to match a stale link.
#[test]
fn nothing_behind_a_symlink_becomes_a_destination() {
    let vault = Vault::new(&[("Index.md", "# Index\n\n- [[legacy/secret/Target.md]]\n")]);

    let outside = vault.root.path().join("outside");
    fs::create_dir_all(outside.join("secret")).unwrap();
    fs::write(outside.join("secret/Target.md"), "# Target\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, vault.at("Elsewhere")).unwrap();
    #[cfg(not(unix))]
    return;

    let plan = vault.plan();
    assert_eq!(plan["summary"]["safe"], 0, "{plan:#}");
    let occurrence = &occurrences(&plan)[0];
    assert_eq!(occurrence["status"], "unresolved");
    assert_eq!(occurrence["candidates"].as_array().unwrap().len(), 0);
    assert!(
        !vault
            .run(&["repair-links", "--dry-run"])
            .ok()
            .stdout
            .contains("outside"),
        "a path outside the vault reached the report"
    );
}

/// `broken-links` now answers from the occurrence scan while `lint` and
/// `query --where broken>0` still count the graph. A vault is entitled to one
/// answer, so the two have to agree on every form a link can take -- which
/// means the scan must find a position for each one, and must not invent an
/// occurrence the graph never saw.
#[test]
fn the_occurrence_scan_and_the_graph_count_the_same_broken_links() {
    let vault = Vault::new(&[
        (
            "Every Form.md",
            "---\n\
             title: Every Form\n\
             related: \"[[Missing Property]]\"\n\
             tags: [\"[[Not A Link]]\"]\n\
             ---\n\n\
             - bare: [[Missing]]\n\
             - anchored: [[Missing Anchor#section]]\n\
             - aliased: [[Missing Alias|see this]]\n\
             - embedded: ![[Missing Embed]]\n\
             - typed: [supports:: [[Missing Field]]]\n\
             - inline link: [text](missing/thing.md)\n\
             - twice: [[Missing]]\n\n\
             None of these is a reference:\n\n\
             ```\n[[Missing In Fence]]\n```\n\n\
             `[[Missing Inline]]` and %% [[Missing Comment]] %%\n",
        ),
        (
            "Notes.org",
            "#+title: Notes\n\n[[file:missing/org.org][Gone]]\n",
        ),
    ]);

    let occurrences = vault.run(&["broken-links", "--format", "json"]).ok().json();
    let occurrences = occurrences.as_array().unwrap().clone();
    let graph = vault
        .run(&["lint", "--check", "broken-links", "--format", "json"])
        .ok()
        .json();

    assert_eq!(
        serde_json::json!(occurrences.len()),
        graph["summary"]["broken_links"],
        "the two disagree:\n{}",
        serde_json::to_string_pretty(&occurrences).unwrap()
    );

    let targets: Vec<&str> = occurrences
        .iter()
        .map(|o| o["target"].as_str().unwrap())
        .collect();
    assert_eq!(
        targets,
        [
            "Missing Property",
            "Missing",
            "Missing Anchor",
            "Missing Alias",
            "Missing Embed",
            "Missing Field",
            "missing/thing",
            "Missing",
            "missing/org",
        ]
    );
}

/// Decoding a target must not be a doorway to the fallbacks this command
/// refuses.
///
/// The resolver reaches a note by stem, by basename and by alias, which is
/// right for reading a vault and is not evidence for an edit. Percent-decoding
/// is a safe basis because it is exact -- but only for the destination the
/// decoded target *names*. Every link below decodes cleanly onto exactly one
/// file and must still not be safe, because the last step to that file is a
/// resemblance rather than a path.
#[test]
fn decoding_cannot_reach_safety_through_a_basename_or_an_alias() {
    let vault = Vault::new(&[
        (
            "Index.md",
            "# Index\n\n\
             - by stem: [[My%20Note]]\n\
             - by stem, with a stale path folded in: [[old%2FMy%20Note]]\n\
             - by alias: [[The%20Old%20Name]]\n\
             - by path, which is the one exact case: [[Docs/My%20Note]]\n",
        ),
        (
            "Docs/My Note.md",
            "---\naliases: [The Old Name]\n---\n\n# My Note\n",
        ),
    ]);

    let plan = vault.plan();
    let verdicts: Vec<(&str, &str, Option<&str>)> = occurrences(&plan)
        .iter()
        .map(|o| {
            (
                o["raw"].as_str().unwrap(),
                o["status"].as_str().unwrap(),
                o["candidates"]
                    .as_array()
                    .unwrap()
                    .first()
                    .map(|c| c["basis"].as_str().unwrap()),
            )
        })
        .collect();

    assert_eq!(
        verdicts,
        [
            // Decoding reaches Docs/My Note.md by stem, so it is a suggestion.
            ("[[My%20Note]]", "ambiguous", Some("basename")),
            // An encoded slash decodes to a path that does not exist; only the
            // basename at the end of it matches anything.
            ("[[old%2FMy%20Note]]", "ambiguous", Some("basename")),
            // An alias is a name the author gave a note, not a location.
            ("[[The%20Old%20Name]]", "unresolved", None),
            // And the one case that is exact: the decoded target is the path.
            ("[[Docs/My%20Note]]", "safe", Some("percent-decoding")),
        ]
    );
    assert_eq!(plan["summary"]["safe"], 1, "{plan:#}");
}

/// A link that resolves is never a repair, whichever of the resolver's rules
/// it resolves by. Rewriting a working link for style is not this command's
/// business, and would churn a vault that is already correct.
#[test]
fn links_that_already_resolve_are_not_repairs() {
    let vault = Vault::new(&[
        (
            "research/survey/index.md",
            "# Survey\n\n\
             - [Summary](../papers/foo/summary.md)\n\
             - [[survey/foo]]\n\
             - [[papers/foo/summary.md]]\n",
        ),
        ("research/survey/foo.md", "# Foo\n"),
        ("research/papers/foo/summary.md", "# Summary\n"),
        ("papers/foo/summary.md", "# Root summary\n"),
    ]);

    let plan = vault.plan();
    assert_eq!(plan["summary"]["occurrences"], 0, "{plan:#}");
}
