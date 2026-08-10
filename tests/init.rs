//! What `knapper init` leaves behind, and whether `knapper daily` works
//! immediately afterwards.
//!
//! These two commands are coupled by the generated config: it names
//! `Templates/daily.md`, excludes that folder from whole-vault reads, and
//! `daily` now fails rather than inventing a body when the template is
//! missing. So `init` writes the template it configures, and the case worth
//! pinning is the sequence -- init, then daily, in a directory that had
//! nothing in it.

use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn knapper(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_knapper"))
        .args(args)
        .current_dir(dir)
        // The config search walks up and then falls back to $HOME, so both
        // are pinned inside the temporary vault.
        .env("HOME", dir)
        .output()
        .expect("knapper runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn empty_vault() -> TempDir {
    tempfile::tempdir().unwrap()
}

const TEMPLATE: &str = "Templates/daily.md";

/// The sequence a new user actually performs, and the one that broke when
/// `daily` stopped inventing a body for a missing template.
#[test]
fn init_then_daily_works_in_a_directory_that_had_nothing_in_it() {
    let dir = empty_vault();

    let init = knapper(dir.path(), &["init"]);
    assert!(init.status.success(), "{}", stderr(&init));
    assert!(
        dir.path().join(TEMPLATE).exists(),
        "init configured {TEMPLATE} without creating it"
    );
    // Both files are reported, because both appeared.
    assert!(
        stdout(&init).contains("knapper.config.md"),
        "{}",
        stdout(&init)
    );
    assert!(stdout(&init).contains(TEMPLATE), "{}", stdout(&init));

    let daily = knapper(dir.path(), &["daily", "--format", "json"]);
    assert!(daily.status.success(), "{}", stderr(&daily));

    let parsed: serde_json::Value = serde_json::from_str(&stdout(&daily)).unwrap();
    let (path, date) = (
        parsed["path"].as_str().unwrap(),
        parsed["date"].as_str().unwrap(),
    );
    // Exactly what the old silent fallback produced, now from a real file.
    assert_eq!(
        std::fs::read_to_string(dir.path().join(path)).unwrap(),
        format!("# {date}\n")
    );
}

/// The generated config excludes the folder it puts the template in, so the
/// template is not counted as one of the user's notes -- which is the whole
/// reason that `exclude` entry is generated.
#[test]
fn the_generated_template_is_not_one_of_the_users_notes() {
    let dir = empty_vault();
    assert!(knapper(dir.path(), &["init"]).status.success());

    let orphans = knapper(dir.path(), &["orphans", "--format", "json"]);
    assert!(orphans.status.success(), "{}", stderr(&orphans));
    assert_eq!(stdout(&orphans).trim(), "[]");
}

/// A template is the user's writing. `--force` replaces knapper's config; it
/// has never had anything to say about a vault's own files.
#[test]
fn an_existing_template_is_never_overwritten() {
    let dir = empty_vault();
    std::fs::create_dir_all(dir.path().join("Templates")).unwrap();
    let mine = "---\nmood:\n---\n\n# {{title}}\n\n## Morning\n";
    std::fs::write(dir.path().join(TEMPLATE), mine).unwrap();

    for args in [vec!["init"], vec!["init", "--force"]] {
        let out = knapper(dir.path(), &args);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        assert_eq!(
            std::fs::read_to_string(dir.path().join(TEMPLATE)).unwrap(),
            mine,
            "{args:?} overwrote the vault's own template"
        );
        // Nothing was created, so nothing is announced.
        assert!(
            !stdout(&out).contains(TEMPLATE),
            "{args:?} claimed to create a template it left alone: {}",
            stdout(&out)
        );
    }

    // And the template that was there is the one `daily` expands.
    let daily = knapper(dir.path(), &["daily", "--format", "json"]);
    assert!(daily.status.success(), "{}", stderr(&daily));
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&daily)).unwrap();
    let body = std::fs::read_to_string(dir.path().join(parsed["path"].as_str().unwrap())).unwrap();
    assert!(body.contains("## Morning"), "{body:?}");
}

/// Refusing to overwrite a config is a refusal to do anything at all. A
/// template appearing next to an untouched config would be a change the
/// error message says was not made.
#[test]
fn a_refused_init_creates_no_template_either() {
    let dir = empty_vault();
    std::fs::write(dir.path().join("knapper.config.md"), "---\n---\n").unwrap();

    let out = knapper(dir.path(), &["init"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("already exists"), "{}", stderr(&out));

    assert!(
        !dir.path().join("Templates").exists(),
        "a refused init created a template folder"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("knapper.config.md")).unwrap(),
        "---\n---\n",
        "a refused init rewrote the config"
    );
}

/// If the template cannot be written, no config is left pointing at it: a
/// vault whose `daily` is broken before it is used is worse than one that has
/// not been initialised at all.
#[cfg(unix)]
#[test]
fn a_template_that_cannot_be_written_leaves_no_config_behind() {
    use std::os::unix::fs::PermissionsExt;

    let dir = empty_vault();
    // A read-only vault root: the template folder cannot be created in it.
    let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(dir.path(), perms).unwrap();

    let out = knapper(dir.path(), &["init"]);
    let readable = std::fs::metadata(dir.path()).unwrap().permissions().mode();

    // Put the directory back before asserting, so a failure here cannot
    // leave an undeletable temporary behind.
    let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(dir.path(), perms).unwrap();
    assert_eq!(readable & 0o500, 0o500, "the test could not set up");

    assert!(
        !out.status.success(),
        "init reported success: {}",
        stdout(&out)
    );
    assert!(
        !dir.path().join("knapper.config.md").exists(),
        "a config was left pointing at a template that could not be written"
    );
    assert!(!dir.path().join("Templates").exists());
}
