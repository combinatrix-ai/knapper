//! What `knapper daily` does when the template it was told to use is not
//! there.
//!
//! This is the case a contract case cannot state: the assertion is about what
//! is *absent* from the filesystem afterwards. A vault that names a template
//! and loses it -- renamed, half-synced, mistyped -- used to get a note
//! containing `# 2026-07-28` and nothing else, silently, every day until
//! somebody noticed. The command now fails and leaves the vault untouched,
//! which is recoverable; a folder of hollow notes is not.

use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

/// A vault holding just a config, with `$HOME` pointed at it so the config
/// search cannot reach the one belonging to whoever is running the suite.
fn vault(config: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("knapper.yaml"), config).unwrap();
    dir
}

fn knapper(vault: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_knapper"))
        .args(args)
        .current_dir(vault)
        .env("HOME", vault)
        .output()
        .expect("knapper runs")
}

fn write(vault: &Path, relative: &str, body: &str) {
    let path = vault.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

const CONFIGURED: &str = "vault_path: .\ndaily_notes:\n  folder: Journal\n  \
                          template: assets/daily.md\n";
const NO_TEMPLATE: &str = "vault_path: .\ndaily_notes:\n  folder: Journal\n";

#[test]
fn a_missing_configured_template_fails_and_creates_nothing() {
    let dir = vault(CONFIGURED);
    let out = knapper(dir.path(), &["daily"]);

    assert!(!out.status.success(), "expected a failure exit");

    // The error has to be actionable without opening the source: which
    // setting, which path, and that nothing happened.
    let stderr = String::from_utf8_lossy(&out.stderr);
    for needle in [
        "daily_notes.template",
        "assets/daily.md",
        "Nothing was written",
    ] {
        assert!(stderr.contains(needle), "stderr lacks {needle:?}: {stderr}");
    }

    // Not the note, and not the folder that would have held it: a failed run
    // leaves the vault exactly as it found it.
    assert!(
        !dir.path().join("Journal").exists(),
        "the daily folder was created anyway"
    );
    let left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["knapper.yaml"], "the vault gained files");
}

/// `--path-only` and `--format json` are output flags, not dry runs: they
/// create the note too, so they fail the same way and leave nothing behind.
#[test]
fn every_output_mode_fails_the_same_way() {
    for args in [
        vec!["daily", "--path-only"],
        vec!["daily", "--format", "json"],
        vec!["daily", "yesterday"],
    ] {
        let dir = vault(CONFIGURED);
        let out = knapper(dir.path(), &args);
        assert!(!out.status.success(), "{args:?} succeeded");
        assert!(
            String::from_utf8_lossy(&out.stdout).trim().is_empty(),
            "{args:?} printed a path for a note it did not create"
        );
        assert!(
            !dir.path().join("Journal").exists(),
            "{args:?} left a folder"
        );
    }
}

/// The failure is about *creating* a note. A vault whose template went
/// missing can still be told where today's existing note is.
#[test]
fn an_existing_daily_note_is_still_reachable_without_its_template() {
    let dir = vault(CONFIGURED);

    // The same vault minus the template setting names today's note for us,
    // so this test does not have to keep its own clock.
    let scratch = vault(NO_TEMPLATE);
    let out = knapper(scratch.path(), &["daily", "--path-only"]);
    let dated = String::from_utf8_lossy(&out.stdout).trim().to_string();

    write(dir.path(), &dated, "# already here\n");

    let out = knapper(dir.path(), &["daily", "--path-only"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), dated);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(&dated)).unwrap(),
        "# already here\n",
        "the existing note was overwritten"
    );
}

/// A vault that named no template never asked for one, so a plain dated note
/// is the whole of what it wants -- and that path is unchanged.
#[test]
fn a_vault_with_no_configured_template_gets_a_plain_dated_note() {
    let dir = vault(NO_TEMPLATE);
    let out = knapper(dir.path(), &["daily", "--format", "json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let path = parsed["path"].as_str().unwrap();
    let date = parsed["date"].as_str().unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join(path)).unwrap(),
        format!("# {date}\n")
    );
}

/// The template that is there is still expanded, so the strict path did not
/// cost the feature it guards.
#[test]
fn a_configured_template_that_exists_is_expanded() {
    let dir = vault(CONFIGURED);
    write(dir.path(), "assets/daily.md", "# {{title}}\n\n- [ ] plan\n");

    let out = knapper(dir.path(), &["daily", "--format", "json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let path = parsed["path"].as_str().unwrap();
    let date = parsed["date"].as_str().unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join(path)).unwrap(),
        format!("# {date}\n\n- [ ] plan\n")
    );
}
