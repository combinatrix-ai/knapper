//! `knapper move` on a directory.
//!
//! The link rewriting itself is pinned in `contract/cases.yaml`, where it
//! belongs: it is what a caller sees. What is here instead is everything that
//! needs a vault built rather than copied -- hidden files, a symlink, a
//! read-only folder, bytes that are not UTF-8 -- and the failure paths, which
//! have to be checked by what they did *not* do to the disk.

use std::fs;
use std::path::{Path, PathBuf};
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
    /// A vault holding these files, with a config so knapper finds it.
    fn new(files: &[(&str, &str)]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let vault = Vault { root };
        vault.write("knapper.config.md", "---\nvault_path: .\n---\n");
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
        self.write_bytes(relative, content.as_bytes());
    }

    fn write_bytes(&self, relative: &str, content: &[u8]) {
        let path = self.at(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.at(relative))
            .unwrap_or_else(|e| panic!("{relative} unreadable: {e}"))
    }

    fn bytes(&self, relative: &str) -> Vec<u8> {
        fs::read(self.at(relative)).unwrap_or_else(|e| panic!("{relative} unreadable: {e}"))
    }

    fn exists(&self, relative: &str) -> bool {
        self.at(relative).symlink_metadata().is_ok()
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

/// Every file under a directory, relative to it.
fn tree(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                stack.push(entry.path());
            } else {
                found.push(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    found.sort();
    found
}

// ------------------------------------------------------------ the subtree --

/// What moves is the directory, not the notes in it: the sidecar, the asset
/// and the dotfile go too, with the shape they had.
#[test]
fn the_whole_subtree_moves_with_its_structure_intact() {
    let vault = Vault::new(&[
        ("Guide/README.md", "# Guide\n"),
        ("Guide/data.jsonl", "{\"a\":1}\n{\"b\":2}\n"),
        ("Guide/.hidden", "not a note, and not visible\n"),
        ("Guide/deep/nested/note.md", "# Nested\n"),
        ("top.md", "See [[Guide/README]].\n"),
    ]);
    // Bytes that are not text at all, so "moved" has to mean moved.
    vault.write_bytes(
        "Guide/assets/diagram.png",
        &[0x89, b'P', b'N', b'G', 0x00, 0xff],
    );

    let before = tree(&vault.at("Guide"));
    let png = vault.bytes("Guide/assets/diagram.png");
    let jsonl = vault.bytes("Guide/data.jsonl");

    let run = vault
        .run(&["move", "Guide", "Archive/", "--format", "json"])
        .ok();
    assert_eq!(run.json()["entries"], 5);
    assert_eq!(run.json()["notes"], 2);

    assert!(!vault.exists("Guide"), "the source directory is gone");
    assert_eq!(tree(&vault.at("Archive/Guide")), before);
    assert_eq!(vault.bytes("Archive/Guide/assets/diagram.png"), png);
    assert_eq!(vault.bytes("Archive/Guide/data.jsonl"), jsonl);
    assert_eq!(
        vault.read("Archive/Guide/.hidden"),
        "not a note, and not visible\n"
    );
    assert_eq!(vault.read("top.md"), "See [[Archive/Guide/README]].\n");
}

/// A hidden note moves like anything else, but nothing scans it, and a report
/// that did not say so would be claiming more than it did.
#[test]
fn notes_the_scanner_cannot_see_are_reported_rather_than_assumed() {
    let vault = Vault::new(&[
        ("Guide/README.md", "# Guide\n"),
        ("Guide/.drafts/scratch.md", "Links [[out]] of sight.\n"),
        ("out.md", "# Out\n"),
    ]);

    let run = vault
        .run(&["move", "Guide", "Archive/", "--format", "json"])
        .ok();
    let warnings = run.json()["warnings"].as_array().unwrap().clone();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].as_str().unwrap().contains("hidden or excluded"),
        "{warnings:?}"
    );
    assert!(vault.exists("Archive/Guide/.drafts/scratch.md"));
}

// -------------------------------------------------------------- rewriting --

/// The parts of a link that are not the path: they belong to whoever wrote
/// them, and a move has no opinion about any of them.
#[test]
fn anchors_labels_titles_and_percent_encoding_survive() {
    let vault = Vault::new(&[
        ("My Guide/My Note.md", "# Note\n"),
        (
            "top.md",
            "- encoded: [a](My%20Guide/My%20Note.md)\n\
             - titled: [b](My%20Guide/My%20Note.md \"The title\")\n\
             - anchored: [c](My%20Guide/My%20Note.md#some-heading)\n\
             - angled: [d](<My Guide/My Note.md>)\n\
             - aliased: [[My Guide/My Note|what I call it]]\n\
             - blocked: [[My Guide/My Note^b12]]\n",
        ),
    ]);

    vault.run(&["move", "My Guide", "Archive/"]).ok();

    assert_eq!(
        vault.read("top.md"),
        "- encoded: [a](Archive/My%20Guide/My%20Note.md)\n\
         - titled: [b](Archive/My%20Guide/My%20Note.md \"The title\")\n\
         - anchored: [c](Archive/My%20Guide/My%20Note.md#some-heading)\n\
         - angled: [d](<Archive/My Guide/My Note.md>)\n\
         - aliased: [[Archive/My Guide/My Note|what I call it]]\n\
         - blocked: [[Archive/My Guide/My Note^b12]]\n"
    );
}

/// Two notes with the same name, and a bare link to each. Only the one that
/// stops resolving is touched.
#[test]
fn a_bare_link_is_requalified_only_when_it_would_otherwise_go_elsewhere() {
    let vault = Vault::new(&[
        ("Guide/README.md", "# Guide readme\n"),
        ("Handbook/README.md", "# Handbook readme\n"),
        // Same depth, and Guide sorts first, so this means Guide's.
        ("top.md", "The readme: [[README]].\n"),
        // Its own neighbour's, and no move changes that.
        ("Handbook/index.md", "Next door: [[README]].\n"),
    ]);

    vault.run(&["move", "Guide", "Deep/Down/"]).ok();

    assert_eq!(
        vault.read("top.md"),
        "The readme: [[Deep/Down/Guide/README]].\n"
    );
    assert_eq!(vault.read("Handbook/index.md"), "Next door: [[README]].\n");
}

/// A note that moves takes its own links with it. The ones that stayed inside
/// the subtree still read the same; the one that left has further to go.
#[test]
fn a_moved_notes_own_links_are_recomputed_only_where_they_have_to_be() {
    let vault = Vault::new(&[
        (
            "Guide/README.md",
            "- inside: [notes](notes.md)\n\
             - inside, deeper: [deep](deep/nested.md)\n\
             - outside: [handbook](../Handbook/README.md)\n\
             - outside, up and over: [top](../top.md)\n\
             - by name: [[Handbook/README]]\n",
        ),
        ("Guide/notes.md", "# Notes\n"),
        ("Guide/deep/nested.md", "# Nested\n"),
        ("Handbook/README.md", "# Handbook\n"),
        ("top.md", "# Top\n"),
    ]);

    vault.run(&["move", "Guide", "Archive/Old/"]).ok();

    assert_eq!(
        vault.read("Archive/Old/Guide/README.md"),
        "- inside: [notes](notes.md)\n\
         - inside, deeper: [deep](deep/nested.md)\n\
         - outside: [handbook](../../../Handbook/README.md)\n\
         - outside, up and over: [top](../../../top.md)\n\
         - by name: [[Handbook/README]]\n"
    );
}

// -------------------------------------------------------------- dry runs --

/// A dry run is what an agent acts on, so the plan has to be exact and the
/// vault has to be untouched.
#[test]
fn a_dry_run_prints_the_whole_plan_and_writes_nothing() {
    let vault = Vault::new(&[
        ("Guide/README.md", "# Guide\n"),
        ("top.md", "[[Guide/README]]\n"),
    ]);
    let before = vault.read("top.md");

    let run = vault
        .run(&["move", "Guide", "Archive/", "--dry-run", "--format", "json"])
        .ok();

    assert_eq!(
        run.json(),
        serde_json::json!({
            "kind": "directory",
            "old_path": "Guide",
            "new_path": "Archive/Guide",
            "dry_run": true,
            "applied": false,
            "blocked": false,
            "entries": 1,
            "notes": 1,
            "files_updated": ["top.md"],
            "links_updated": 1,
            "unsupported_links": [],
            "warnings": [],
            "moves": [{"from": "Guide/README.md", "to": "Archive/Guide/README.md"}],
            "edits": [{
                "file": "top.md",
                "new_file": "top.md",
                "links": 1,
                "changes": [{
                    "line": 1,
                    "before": "[[Guide/README]]",
                    "after": "[[Archive/Guide/README]]",
                }],
            }],
        })
    );

    assert_eq!(vault.read("top.md"), before);
    assert!(vault.exists("Guide/README.md"));
    assert!(!vault.exists("Archive"));
}

// ------------------------------------------------------------ refusals --

fn guide_vault() -> Vault {
    Vault::new(&[
        ("Guide/README.md", "# Guide\n"),
        ("top.md", "[[Guide/README]]\n"),
    ])
}

/// Each of these has to fail before anything is written, so every case
/// asserts the vault as well as the exit status.
#[test]
fn a_directory_move_that_cannot_be_safe_is_refused_before_anything_moves() {
    for (files, args, expected) in [
        // An existing target: v1 does not merge two directories.
        (
            vec![("Archive/Guide/other.md", "# Other\n")],
            vec!["move", "Guide", "Archive/"],
            "already exists",
        ),
        // Moving a directory into itself.
        (
            vec![],
            vec!["move", "Guide", "Guide/inner"],
            "inside the source",
        ),
        (vec![], vec!["move", "Guide", "Guide"], "inside the source"),
        // Where it already is.
        (vec![], vec!["move", "Guide", "."], "already there"),
    ] {
        let vault = guide_vault();
        for (path, content) in &files {
            vault.write(path, content);
        }
        let run = vault.run(&args);
        assert_eq!(run.code, 1, "{args:?} should fail\n{}", run.stdout);
        assert!(
            run.stderr.contains(expected),
            "{args:?}: stderr lacks {expected:?}\n{}",
            run.stderr
        );
        assert!(vault.exists("Guide/README.md"), "{args:?} moved the source");
        assert_eq!(vault.read("top.md"), "[[Guide/README]]\n", "{args:?}");
    }
}

#[test]
fn a_directory_outside_the_vault_is_refused() {
    let vault = guide_vault();
    let outside = vault.root.path().join("outside");
    fs::create_dir_all(outside.join("Sub")).unwrap();
    fs::write(outside.join("Sub/note.md"), "# Outside\n").unwrap();

    for source in [
        "../outside/Sub".to_string(),
        outside.join("Sub").to_string_lossy().into_owned(),
    ] {
        let run = vault.run(&["move", &source, "Archive/"]);
        assert_eq!(run.code, 1, "{source} should fail\n{}", run.stdout);
        assert!(
            run.stderr.contains("Outside the vault"),
            "{source}: {}",
            run.stderr
        );
        assert!(outside.join("Sub/note.md").exists());
        assert!(!vault.exists("Archive"));
    }
}

#[cfg(unix)]
#[test]
fn a_symlinked_directory_is_refused() {
    let vault = guide_vault();
    std::os::unix::fs::symlink(vault.at("Guide"), vault.at("Shortcut")).unwrap();

    let run = vault.run(&["move", "Shortcut", "Archive/"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stderr.contains("symlink"), "{}", run.stderr);
    assert!(vault.exists("Shortcut"));
    assert!(vault.exists("Guide/README.md"));
}

/// An excluded subtree is one knapper was told not to read. Moving into or
/// out of it would mean rewriting links against a vault it cannot see.
#[test]
fn an_excluded_source_or_destination_is_refused() {
    for (source, destination) in [("Logs/Old", "Archive/"), ("Guide", "Logs/")] {
        let vault = guide_vault();
        vault.write(
            "knapper.config.md",
            "---\nvault_path: .\nexclude:\n  - Logs\n---\n",
        );
        vault.write("Logs/Old/note.md", "# Old\n");

        let run = vault.run(&["move", source, destination]);
        assert_eq!(run.code, 1, "{source} -> {destination}\n{}", run.stdout);
        assert!(
            run.stderr.contains("excluded"),
            "{source} -> {destination}: {}",
            run.stderr
        );
        assert!(vault.exists("Logs/Old/note.md"));
        assert!(vault.exists("Guide/README.md"));
    }
}

/// A note nobody can read is a note whose links nobody can check. Finding
/// that out half way through the writes is what the plan-first design exists
/// to prevent, so it is a refusal rather than a warning.
#[test]
fn a_note_that_cannot_be_read_stops_the_move() {
    let vault = guide_vault();
    vault.write_bytes("broken.md", &[0xff, 0xfe, 0x00, 0x9f]);

    let run = vault.run(&["move", "Guide", "Archive/"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stderr.contains("broken.md"), "{}", run.stderr);
    assert!(vault.exists("Guide/README.md"));
    assert!(!vault.exists("Archive"));
}

/// A symlink is a hole in every path check that works on the text of a path.
/// `Elsewhere/Guide` normalizes to something perfectly vault-relative, and
/// `rename` would follow it clean out of the vault -- moving the directory
/// somewhere knapper can no longer see, while rewriting every inbound link to
/// a path that does not exist.
#[cfg(unix)]
#[test]
fn a_destination_reached_through_a_symlink_is_refused() {
    let vault = guide_vault();
    let outside = vault.root.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, vault.at("Elsewhere")).unwrap();

    let run = vault.run(&["move", "Guide", "Elsewhere/"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stderr.contains("symlink"), "{}", run.stderr);

    assert!(vault.exists("Guide/README.md"), "the source stayed put");
    assert_eq!(vault.read("top.md"), "[[Guide/README]]\n");
    assert_eq!(
        tree(&outside),
        Vec::<String>::new(),
        "nothing left the vault"
    );
}

/// The same hole, one level up: the destination's parent is real, but the
/// grandparent it hangs off is not.
#[cfg(unix)]
#[test]
fn a_destination_whose_parent_is_a_symlink_is_refused() {
    let vault = guide_vault();
    let outside = vault.root.path().join("outside");
    fs::create_dir_all(outside.join("Deep")).unwrap();
    std::os::unix::fs::symlink(&outside, vault.at("Elsewhere")).unwrap();

    let run = vault.run(&["move", "Guide", "Elsewhere/Deep/"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stderr.contains("symlink"), "{}", run.stderr);
    assert!(vault.exists("Guide/README.md"));
    assert_eq!(tree(&outside.join("Deep")), Vec::<String>::new());
}

/// And a source that only reaches its directory through one.
#[cfg(unix)]
#[test]
fn a_source_reached_through_a_symlinked_parent_is_refused() {
    let vault = guide_vault();
    let outside = vault.root.path().join("outside");
    fs::create_dir_all(outside.join("Sub")).unwrap();
    fs::write(outside.join("Sub/note.md"), "# Outside\n").unwrap();
    std::os::unix::fs::symlink(&outside, vault.at("Elsewhere")).unwrap();

    let run = vault.run(&["move", "Elsewhere/Sub", "Archive/"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stderr.contains("symlink"), "{}", run.stderr);
    assert!(outside.join("Sub/note.md").exists());
    assert!(!vault.exists("Archive"));
}

// ------------------------------------------------------------ temp files --

/// The temp file a rewrite goes through is created exclusively, under a name
/// of its own. A fixed one would open -- then, at the rename, unlink --
/// whatever a user or another tool already had there.
#[test]
fn a_file_that_looks_like_a_temp_file_is_never_touched() {
    let vault = Vault::new(&[
        ("Guide/README.md", "# Guide\n"),
        ("top.md", "[[Guide/README]]\n"),
    ]);
    // Exactly the name the first implementation used.
    vault.write(".top.md.knapper-tmp", "somebody else's file\n");

    vault.run(&["move", "Guide", "Archive/"]).ok();

    assert_eq!(vault.read("top.md"), "[[Archive/Guide/README]]\n");
    assert_eq!(
        vault.read(".top.md.knapper-tmp"),
        "somebody else's file\n",
        "a bystander file was overwritten or removed"
    );
}

/// A successful move leaves no temp file behind, whatever it was called.
#[test]
fn no_temp_file_survives_a_successful_move() {
    let vault = Vault::new(&[
        ("Guide/README.md", "# Guide\n"),
        ("top.md", "[[Guide/README]]\n"),
        ("Notes/index.md", "[[Guide/README]]\n"),
    ]);

    vault.run(&["move", "Guide", "Archive/"]).ok();

    let left: Vec<String> = tree(&vault.path())
        .into_iter()
        .filter(|p| p.contains("knapper-") || p.ends_with(".tmp"))
        .collect();
    assert_eq!(left, Vec::<String>::new(), "temp files were left behind");
}

/// A note somebody made read-only, or opened up for a shared vault, should
/// still be that way after knapper rewrites a link in it. The rewrite goes
/// through a fresh file, which starts from this process's umask instead.
#[cfg(unix)]
#[test]
fn a_notes_permissions_survive_a_rewrite() {
    use std::os::unix::fs::PermissionsExt;

    let vault = Vault::new(&[
        ("Guide/README.md", "# Guide\n"),
        ("top.md", "[[Guide/README]]\n"),
        ("other.md", "[[Guide/README]]\n"),
    ]);
    fs::set_permissions(vault.at("top.md"), fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(vault.at("other.md"), fs::Permissions::from_mode(0o664)).unwrap();

    vault.run(&["move", "Guide", "Archive/"]).ok();

    assert_eq!(vault.read("top.md"), "[[Archive/Guide/README]]\n");
    for (path, expected) in [("top.md", 0o600), ("other.md", 0o664)] {
        let mode = fs::metadata(vault.at(path)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, expected, "{path} came back as {mode:o}");
    }
}

/// `all_notes` follows a symlink to decide a file is a note, so a symlinked
/// note is one knapper reads and rewrites. Renaming a temp file over the link
/// would replace it with a regular file -- silently detaching a note the user
/// had deliberately shared into the vault -- so the rewrite goes through the
/// link to the file it names, exactly as `fs::write` does everywhere else.
///
/// Both directions matter: a referrer that stays put, and one that rides
/// along inside the moved subtree and has to be found at its new path.
#[cfg(unix)]
#[test]
fn a_symlinked_referrer_stays_a_symlink_and_its_target_is_updated() {
    let vault = Vault::new(&[
        ("Guide/README.md", "# Guide\n"),
        ("Handbook/README.md", "# Handbook\n"),
    ]);

    // The real files live outside the vault; the vault holds links to them.
    let external = vault.root.path().join("external");
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join("outside.md"), "Points in: [[Guide/README]]\n").unwrap();
    fs::write(
        external.join("inside.md"),
        "Points out: [h](../Handbook/README.md)\n",
    )
    .unwrap();

    // One referrer that stays where it is...
    std::os::unix::fs::symlink(external.join("outside.md"), vault.at("linked.md")).unwrap();
    // ...and one that travels with the directory being moved.
    std::os::unix::fs::symlink(external.join("inside.md"), vault.at("Guide/linked.md")).unwrap();

    vault.run(&["move", "Guide", "Archive/"]).ok();

    // Still links, not files that used to be links.
    for path in ["linked.md", "Archive/Guide/linked.md"] {
        let meta = fs::symlink_metadata(vault.at(path)).unwrap();
        assert!(
            meta.file_type().is_symlink(),
            "{path} was replaced by a regular file"
        );
    }

    // And what they point at is what changed.
    assert_eq!(
        fs::read_to_string(external.join("outside.md")).unwrap(),
        "Points in: [[Archive/Guide/README]]\n"
    );
    assert_eq!(
        fs::read_to_string(external.join("inside.md")).unwrap(),
        // Read through Archive/Guide/linked.md, so one level further out.
        "Points out: [h](../../Handbook/README.md)\n",
        "the moved referrer's new path was not mapped correctly"
    );

    // Reading through the links agrees, and no temp file was left anywhere.
    assert_eq!(
        vault.read("linked.md"),
        "Points in: [[Archive/Guide/README]]\n"
    );
    let left: Vec<String> = tree(&external)
        .into_iter()
        .filter(|p| p.contains("knapper-"))
        .collect();
    assert_eq!(left, Vec::<String>::new(), "temp files were left behind");
}

// ----------------------------------------------------------------- rollback --

/// A write that fails part-way has to leave the vault as it was: the notes it
/// already rewrote go back, and so does the directory.
#[cfg(unix)]
#[test]
fn a_failed_write_puts_everything_back() {
    use std::os::unix::fs::PermissionsExt;

    let vault = Vault::new(&[
        ("Guide/README.md", "# Guide\n"),
        // Edits are applied in path order, so this one lands first...
        ("a.md", "[[Guide/README]]\n"),
        // ...and this one cannot be written at all.
        ("locked/b.md", "[[Guide/README]]\n"),
    ]);

    let locked = vault.at("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    // Running as root defeats the point of the fixture.
    if fs::write(locked.join("probe"), "x").is_ok() {
        fs::remove_file(locked.join("probe")).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipped: this user can write to a read-only directory");
        return;
    }

    let run = vault.run(&["move", "Guide", "Archive/"]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stderr.contains("rolled back"), "{}", run.stderr);

    assert!(vault.exists("Guide/README.md"), "the directory came back");
    assert!(!vault.exists("Archive/Guide"));
    assert!(
        !vault.exists("Archive"),
        "a directory it created was removed"
    );
    assert_eq!(
        vault.read("a.md"),
        "[[Guide/README]]\n",
        "the note it wrote was restored"
    );
    assert_eq!(vault.read("locked/b.md"), "[[Guide/README]]\n");
    assert_eq!(tree(&vault.at("locked")), ["b.md"], "no temp file was left");
}

/// When the rollback itself cannot finish, saying "nothing was changed" would
/// be a lie about the vault. `a.md` is read-only in a writable folder: the
/// rewrite lands (rename does not need the file to be writable), and the
/// restore afterwards does not.
#[cfg(unix)]
#[test]
fn a_rollback_that_cannot_finish_says_so() {
    use std::os::unix::fs::PermissionsExt;

    let vault = Vault::new(&[
        ("Guide/README.md", "# Guide\n"),
        ("a.md", "[[Guide/README]]\n"),
        ("locked/b.md", "[[Guide/README]]\n"),
    ]);

    let locked = vault.at("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    if fs::write(locked.join("probe"), "x").is_ok() {
        fs::remove_file(locked.join("probe")).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipped: this user can write to a read-only directory");
        return;
    }
    fs::set_permissions(vault.at("a.md"), fs::Permissions::from_mode(0o444)).unwrap();

    let run = vault.run(&["move", "Guide", "Archive/"]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(vault.at("a.md"), fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(
        run.stderr.contains("rollback was incomplete"),
        "the failure was reported as a clean rollback: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("a.md"),
        "the note it could not restore is not named: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("Nothing was changed"),
        "it claimed the vault was untouched: {}",
        run.stderr
    );

    // What it *could* undo, it did.
    assert!(vault.exists("Guide/README.md"), "the directory came back");
    assert!(!vault.exists("Archive"));
    // And what it could not is exactly what the message said.
    assert_eq!(
        vault.read("a.md"),
        "[[Archive/Guide/README]]\n",
        "a.md is the file the message warned about"
    );
}

// -------------------------------------------------------------- encoding --

/// A path is a filename, not a URL, and the characters that end an inline
/// link have to go back out encoded. `Guide(x)` written plainly would give
/// `[a](Archive/Guide(x)/Note.md)`, whose destination stops at the first `)`.
#[test]
fn reserved_characters_in_a_rewritten_href_are_re_encoded() {
    let vault = Vault::new(&[
        ("Guide(x)/Note.md", "# Note\n"),
        ("Guide#1/Note.md", "# Hashed\n"),
        (
            "top.md",
            "- parens: [a](Guide%28x%29/Note.md)\n\
             - parens, anchored: [b](Guide%28x%29/Note.md#a-heading)\n\
             - hash in the folder name: [c](Guide%231/Note.md)\n",
        ),
    ]);

    vault.run(&["move", "Guide(x)", "Archive/"]).ok();
    vault.run(&["move", "Guide#1", "Archive/"]).ok();

    assert_eq!(
        vault.read("top.md"),
        "- parens: [a](Archive/Guide%28x%29/Note.md)\n\
         - parens, anchored: [b](Archive/Guide%28x%29/Note.md#a-heading)\n\
         - hash in the folder name: [c](Archive/Guide%231/Note.md)\n"
    );
    assert!(vault.exists("Archive/Guide(x)/Note.md"));
    assert!(vault.exists("Archive/Guide#1/Note.md"));
}

/// The angle form is scanned through the same decode as any other href, so
/// it has to be rendered through the matching encode. Left alone, `Note%.md`
/// is not well-formed percent-encoding and `Guide#1` reads as an anchor,
/// which would swallow the rest of the path.
#[test]
fn the_angle_form_is_re_encoded_where_it_has_to_be() {
    let vault = Vault::new(&[
        ("Guide#1/Note%.md", "# Note\n"),
        (
            "top.md",
            "- angled: [x](<Guide%231/Note%25.md>)\n\
             - angled, anchored: [y](<Guide%231/Note%25.md#a-heading>)\n\
             - angled, spaced: [z](<Guide%231/Note%25.md>)\n",
        ),
    ]);

    vault.run(&["move", "Guide#1", "Archive/"]).ok();

    assert_eq!(
        vault.read("top.md"),
        "- angled: [x](<Archive/Guide%231/Note%25.md>)\n\
         - angled, anchored: [y](<Archive/Guide%231/Note%25.md#a-heading>)\n\
         - angled, spaced: [z](<Archive/Guide%231/Note%25.md>)\n"
    );
    assert!(vault.exists("Archive/Guide#1/Note%.md"));
}

/// The angle form keeps what it is for: a space stays a space.
#[test]
fn the_angle_form_keeps_its_readable_spaces() {
    let vault = Vault::new(&[
        ("My Guide/My Note.md", "# Note\n"),
        ("top.md", "[d](<My Guide/My Note.md>)\n"),
    ]);

    vault.run(&["move", "My Guide", "Arch (v1)/"]).ok();

    assert_eq!(
        vault.read("top.md"),
        "[d](<Arch (v1)/My Guide/My Note.md>)\n"
    );
}

/// The other direction: a plain link that acquires a reserved character
/// purely because the directory it now lives under has one in its name.
#[test]
fn a_plain_href_is_encoded_when_the_destination_needs_it() {
    let vault = Vault::new(&[
        ("Guide/Note.md", "# Note\n"),
        ("top.md", "[a](Guide/Note.md) and [[Guide/Note]]\n"),
    ]);

    vault.run(&["move", "Guide", "Arch (v1)/"]).ok();

    assert_eq!(
        vault.read("top.md"),
        // The wikilink needs none of this: its own syntax is not URL-like.
        "[a](Arch%20%28v1%29/Guide/Note.md) and [[Arch (v1)/Guide/Note]]\n"
    );
    assert!(vault.exists("Arch (v1)/Guide/Note.md"));
}

// --------------------------------------------------------------------- org --

/// knapper reads org but does not rewrite it. An inbound org link into a
/// directory that is about to move is therefore a link this move would
/// silently break, which is the one case worth refusing outright.
#[test]
fn an_inbound_org_link_blocks_the_move_until_it_is_acknowledged() {
    let files = &[
        ("Guide/README.md", "# Guide\n"),
        (
            "notes.org",
            "* Notes\n\n[[file:Guide/README.md][the guide]]\n",
        ),
        ("top.md", "[[Guide/README]]\n"),
    ];

    // Refused, and nothing happened.
    let vault = Vault::new(files);
    let run = vault.run(&["move", "Guide", "Archive/"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stderr.contains("org link"), "{}", run.stderr);
    assert!(run.stdout.contains("Nothing was written"), "{}", run.stdout);
    assert!(vault.exists("Guide/README.md"));
    assert_eq!(vault.read("top.md"), "[[Guide/README]]\n");

    // A dry run reports them instead of failing, so they can be seen first.
    let vault = Vault::new(files);
    let run = vault
        .run(&["move", "Guide", "Archive/", "--dry-run", "--format", "json"])
        .ok();
    let plan = run.json();
    assert_eq!(plan["blocked"], true);
    let unsupported = plan["unsupported_links"].as_array().unwrap();
    assert_eq!(unsupported.len(), 1, "{unsupported:?}");
    assert_eq!(unsupported[0]["file"], "notes.org");
    assert!(vault.exists("Guide/README.md"));

    // Acknowledged, the move goes ahead and still says what it left behind.
    let vault = Vault::new(files);
    let run = vault
        .run(&[
            "move",
            "Guide",
            "Archive/",
            "--allow-broken-org-links",
            "--format",
            "json",
        ])
        .ok();
    assert_eq!(run.json()["applied"], true);
    assert_eq!(run.json()["unsupported_links"].as_array().unwrap().len(), 1);
    assert!(vault.exists("Archive/Guide/README.md"));
    assert_eq!(vault.read("top.md"), "[[Archive/Guide/README]]\n");
}

/// An org ID does not name a place, so moving the file it lives in breaks
/// nothing and there is nothing to report.
#[test]
fn an_org_link_that_survives_the_move_is_not_reported() {
    let vault = Vault::new(&[
        (
            "Guide/README.org",
            ":PROPERTIES:\n:ID: abc-123\n:END:\n#+TITLE: Guide\n",
        ),
        ("notes.org", "* Notes\n\n[[id:abc-123][the guide]]\n"),
    ]);

    let run = vault
        .run(&["move", "Guide", "Archive/", "--format", "json"])
        .ok();
    assert_eq!(run.json()["unsupported_links"].as_array().unwrap().len(), 0);
    assert!(vault.exists("Archive/Guide/README.org"));
}
