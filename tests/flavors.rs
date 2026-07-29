//! Compatibility, pinned per ecosystem.
//!
//! `README.md` and `docs/COMPATIBILITY.md` name the flavors knapper works
//! with. This file is what makes those names mean something: each row runs
//! the real commands against a tiny fixture vault and pins the result, so a
//! change in link or task handling shows up here as a diff instead of as a
//! silent regression in somebody's vault.
//!
//! The numbers are a description of current behaviour, not an aspiration.
//! Where knapper is wrong today the expected value records the wrong answer
//! and the comment says so, so that fixing it is a visible, deliberate edit.

use std::path::{Path, PathBuf};
use std::process::Command;

/// files, links, broken, tasks.
type Counts = (usize, usize, usize, usize);

/// `files` excludes knapper.config.md, which is knapper's own settings
/// rather than one of the user's notes.
const EXPECTED: &[(&str, Counts)] = &[
    // Fully supported. The obsidian fixture also proves that links inside a
    // code fence, an inline span and a %%comment%% are excluded (they would
    // otherwise show up as broken), and that [[an alias]] resolves.
    ("obsidian", (3, 2, 0, 4)),
    ("foam", (2, 2, 0, 0)),
    ("zettelkasten", (2, 2, 0, 0)),
    ("markdown-plain", (3, 3, 0, 0)),
    // Partially supported. Each shortfall is a known gap, described in
    // tests/fixtures/flavors/README.md.
    //
    // dendron: all three link forms resolve, including the extensionless
    // "[d](proj.knapper.design)". The count is edges, not link occurrences,
    // and all three point at the same note, so it stays at 2.
    ("dendron", (2, 2, 0, 0)),
    // dataview: the wikilink inside an inline field is found; the field
    // itself, including "[source:: path.md]", is not.
    ("dataview", (2, 1, 0, 0)),
    // logseq: the fixture declares flavor: logseq, which turns on bare TODO /
    // DOING / LATER keywords and folds SCHEDULED: into the scheduled date.
    // "((uuid))" block references are ignored rather than modelled.
    ("logseq", (3, 2, 0, 3)),
    // roam: macros do not leak phantom links and [[[[x]]]] parses, so the two
    // remaining broken targets are genuinely missing pages. {{[[TODO]]}} is
    // unambiguous and is read as a task with no config.
    ("roam", (3, 2, 2, 1)),
    // orgmode: .org files are scanned by their own reader. Both link forms
    // resolve, including [[*Method]], which org resolves against headings
    // across every file -- there is no Obsidian equivalent.
    ("orgmode", (2, 2, 0, 1)),
];

fn flavors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flavors")
}

fn run(vault: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_knapper"))
        .args(args)
        .current_dir(vault)
        .output()
        .expect("knapper runs");
    assert!(
        output.status.success(),
        "knapper {args:?} failed in {}: {}",
        vault.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn json(vault: &Path, args: &[&str]) -> serde_json::Value {
    serde_json::from_str(&run(vault, args)).expect("output is JSON")
}

fn measure(flavor: &str) -> Counts {
    let vault = flavors_dir().join(flavor);

    let files = run(&vault, &["query", "--format", "paths"])
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();

    let links = json(
        &vault,
        &["query", "--field", "outlinks", "--format", "json"],
    )
    .as_array()
    .map(|rows| {
        rows.iter()
            .filter_map(|r| r.get("outlinks").and_then(serde_json::Value::as_u64))
            .sum::<u64>() as usize
    })
    .unwrap_or(0);

    // broken-links groups its findings by source file, so the count that
    // means "how many links point nowhere" is the sum, not the entry count.
    let broken = json(&vault, &["broken-links", "--format", "json"])
        .as_array()
        .map(|files| {
            files
                .iter()
                .filter_map(|f| f.get("broken_links").and_then(serde_json::Value::as_array))
                .map(Vec::len)
                .sum()
        })
        .unwrap_or(0);

    let tasks = json(&vault, &["tasks", "--format", "json"])
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);

    (files, links, broken, tasks)
}

#[test]
fn flavor_behaviour_is_pinned() {
    let mut wrong = Vec::new();
    for (flavor, expected) in EXPECTED {
        let got = measure(flavor);
        if got != *expected {
            wrong.push(format!(
                "{flavor}: expected (files, links, broken, tasks) = {expected:?}, got {got:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The flavors the README claims outright must resolve cleanly.
#[test]
fn fully_supported_flavors_have_no_broken_links() {
    for flavor in ["obsidian", "foam", "zettelkasten", "markdown-plain"] {
        let (_, _, broken, _) = measure(flavor);
        assert_eq!(broken, 0, "{flavor} has broken links");
    }
}

/// A flavor with a fixture but no row here would be an untested
/// compatibility claim.
#[test]
fn every_flavor_fixture_has_an_expectation() {
    let mut on_disk: Vec<String> = std::fs::read_dir(flavors_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();

    let mut listed: Vec<String> = EXPECTED.iter().map(|(f, _)| f.to_string()).collect();
    listed.sort();

    assert_eq!(on_disk, listed);
}
