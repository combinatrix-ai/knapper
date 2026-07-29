//! Run the contract cases against the knapper binary.
//!
//! The cases live in `contract/cases.yaml` as data, so this file only knows
//! how to invoke a command line and check assertions. Nothing here is tied to
//! how the implementation is arranged, which is the point: another
//! implementation could be checked against the same file.
//!
//! `cargo test` builds the binary and passes its path in, so there is nothing
//! to install first. See `tests/contract/README.md`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    vault: String,
    args: Vec<String>,
    #[serde(default)]
    sorted: bool,
    #[serde(default)]
    expect: Expect,
}

#[derive(Debug, Default, Deserialize)]
struct Expect {
    #[serde(default)]
    exit: i32,
    json: Option<serde_json::Value>,
    json_length: Option<usize>,
    #[serde(default)]
    json_contains: Vec<serde_json::Value>,
    #[serde(default)]
    json_excludes: Vec<serde_json::Value>,
    #[serde(default)]
    json_at: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    stdout_contains: Vec<String>,
    #[serde(default)]
    stdout_excludes: Vec<String>,
    #[serde(default)]
    stderr_contains: Vec<String>,
    #[serde(default)]
    stderr_excludes: Vec<String>,
    lines: Option<Vec<String>>,
    #[serde(default)]
    file: std::collections::BTreeMap<String, FileChecks>,
}

#[derive(Debug, Default, Deserialize)]
struct FileChecks {
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    excludes: Vec<String>,
}

/// Ecosystem fixtures answer "does knapper read Foam / Logseq / org?".
/// Behaviour fixtures answer "does knapper do the right thing?" and belong to
/// no ecosystem. Both are just vaults as far as a case is concerned.
fn fixture_roots() -> [PathBuf; 2] {
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    [here.join("flavors"), here.join("behaviour")]
}

fn fixture(name: &str) -> PathBuf {
    fixture_roots()
        .into_iter()
        .map(|root| root.join(name))
        .find(|p| p.is_dir())
        .unwrap_or_else(|| panic!("no fixture named {name:?}"))
}

fn cases() -> Vec<Case> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contract/cases.yaml");
    let text = fs::read_to_string(&path).expect("cases.yaml is readable");
    serde_yaml::from_str(&text).expect("cases.yaml parses")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Follow a dotted path into parsed JSON. Numeric parts index arrays.
fn dotted<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for part in path.split('.') {
        current = match part.parse::<usize>() {
            Ok(index) => current.get(index)?,
            Err(_) => current.get(part)?,
        };
    }
    Some(current)
}

fn sorted_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            let mut items: Vec<String> = items.iter().map(|i| i.to_string()).collect();
            items.sort();
            serde_json::Value::Array(
                items
                    .iter()
                    .map(|s| serde_json::from_str(s).unwrap())
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

/// Every failure carries the whole invocation, because a bare assertion
/// message is not enough to act on.
fn check(case: &Case, vault: &Path) -> Result<(), String> {
    let output = Command::new(env!("CARGO_BIN_EXE_knapper"))
        .args(&case.args)
        .current_dir(vault)
        .output()
        .map_err(|e| format!("could not run knapper: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let code = output.status.code().unwrap_or(-1);

    let context = format!(
        "\n$ knapper {}\n--- exit {code} ---\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        case.args.join(" ")
    );
    let fail = |what: String| Err(format!("{what}{context}"));

    let e = &case.expect;
    if code != e.exit {
        return fail(format!("expected exit {}, got {code}", e.exit));
    }

    let needs_json = e.json.is_some()
        || e.json_length.is_some()
        || !e.json_contains.is_empty()
        || !e.json_excludes.is_empty()
        || !e.json_at.is_empty();
    let parsed: Option<serde_json::Value> = if needs_json {
        match serde_json::from_str(&stdout) {
            Ok(value) => Some(value),
            Err(err) => return fail(format!("stdout was not JSON: {err}")),
        }
    } else {
        None
    };

    if let (Some(expected), Some(actual)) = (&e.json, &parsed) {
        let (expected, actual) = if case.sorted {
            (sorted_json(expected), sorted_json(actual))
        } else {
            (expected.clone(), actual.clone())
        };
        if expected != actual {
            return fail(format!(
                "json mismatch\nexpected: {expected}\nactual:   {actual}"
            ));
        }
    }
    if let (Some(length), Some(actual)) = (e.json_length, &parsed) {
        let got = actual.as_array().map(Vec::len).unwrap_or(0);
        if got != length {
            return fail(format!("expected {length} items, got {got}"));
        }
    }
    if let Some(actual) = &parsed {
        let items = actual.as_array().cloned().unwrap_or_default();
        for entry in &e.json_contains {
            if !items.contains(entry) {
                return fail(format!("json lacks {entry}"));
            }
        }
        for entry in &e.json_excludes {
            if items.contains(entry) {
                return fail(format!("json still has {entry}"));
            }
        }
        for (path, expected) in &e.json_at {
            match dotted(actual, path) {
                Some(got) if got == expected => {}
                Some(got) => return fail(format!("at {path}: expected {expected}, got {got}")),
                None => return fail(format!("at {path}: nothing there")),
            }
        }
    }

    for needle in &e.stdout_contains {
        if !stdout.contains(needle) {
            return fail(format!("stdout lacks {needle:?}"));
        }
    }
    for needle in &e.stdout_excludes {
        if stdout.contains(needle) {
            return fail(format!("stdout still has {needle:?}"));
        }
    }
    for needle in &e.stderr_contains {
        if !stderr.contains(needle) {
            return fail(format!("stderr lacks {needle:?}"));
        }
    }
    for needle in &e.stderr_excludes {
        if stderr.contains(needle) {
            return fail(format!("stderr still has {needle:?}"));
        }
    }

    if let Some(expected) = &e.lines {
        let mut actual: Vec<String> = stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect();
        let mut expected = expected.clone();
        if case.sorted {
            actual.sort();
            expected.sort();
        }
        if actual != expected {
            return fail(format!(
                "lines mismatch\nexpected: {expected:?}\nactual:   {actual:?}"
            ));
        }
    }

    for (relative, checks) in &e.file {
        let body = match fs::read_to_string(vault.join(relative)) {
            Ok(body) => body,
            Err(err) => return fail(format!("{relative} unreadable: {err}")),
        };
        for needle in &checks.contains {
            if !body.contains(needle) {
                return fail(format!("{relative} lacks {needle:?}"));
            }
        }
        for needle in &checks.excludes {
            if body.contains(needle) {
                return fail(format!("{relative} still has {needle:?}"));
            }
        }
    }

    Ok(())
}

#[test]
fn contract_cases_hold() {
    let cases = cases();
    assert!(!cases.is_empty(), "no cases loaded");

    let mut failures = Vec::new();
    for case in &cases {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        copy_dir(&fixture(&case.vault), &vault);

        if let Err(report) = check(case, &vault) {
            failures.push(format!("\n=== {} ===\n{report}", case.name));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} contract cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// A fixture with no contract case is an untested claim.
#[test]
fn every_fixture_is_covered() {
    let on_disk: BTreeSet<String> = fixture_roots()
        .iter()
        .flat_map(|root| fs::read_dir(root).unwrap())
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let covered: BTreeSet<String> = cases().into_iter().map(|c| c.vault).collect();

    assert_eq!(
        on_disk,
        covered,
        "uncovered fixtures: {:?}",
        on_disk.difference(&covered).collect::<Vec<_>>()
    );
}

#[test]
fn case_names_are_unique() {
    let cases = cases();
    let unique: BTreeSet<&String> = cases.iter().map(|c| &c.name).collect();
    assert_eq!(unique.len(), cases.len(), "duplicate case name");
}
