//! Exercise the shipped installer offline with real archives and a fake transport.
#![cfg(unix)]
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn installer_checks_archive_before_atomic_replacement() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("stage/knapper")).unwrap();
    fs::create_dir(root.join("transport")).unwrap();
    fs::create_dir(root.join("bin")).unwrap();
    fs::write(
        root.join("stage/knapper/knapper"),
        "#!/bin/sh\necho knapper 0.1.0\n",
    )
    .unwrap();
    let asset = "knapper-v0.1.0-aarch64-apple-darwin.tar.gz";
    assert!(Command::new("tar")
        .args(["-czf"])
        .arg(root.join(asset))
        .arg("-C")
        .arg(root.join("stage"))
        .arg("knapper")
        .status()
        .unwrap()
        .success());
    let digest = Command::new("shasum")
        .args(["-a", "256"])
        .arg(root.join(asset))
        .output()
        .unwrap();
    let digest = String::from_utf8(digest.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    fs::write(
        root.join("knapper-v0.1.0-checksums.txt"),
        format!("{digest}  {asset}\n"),
    )
    .unwrap();
    let curl = root.join("transport/curl");
    fs::write(&curl, "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n case \"$1\" in --output) out=\"$2\"; shift 2;; *) url=\"$1\"; shift;; esac\ndone\ncp \"$FIXTURES/${url##*/}\" \"$out\"\n").unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
    let run = |expected: &str| {
        Command::new("sh")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"))
            .args([
                "--version",
                "v0.1.0",
                "--target",
                "aarch64-apple-darwin",
                "--no-skill",
                "--expect-sha256",
                expected,
                "--bin-dir",
            ])
            .arg(root.join("bin"))
            .env("FIXTURES", root)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    root.join("transport").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .output()
            .unwrap()
    };
    fs::write(root.join("bin/knapper"), "old binary").unwrap();
    let rejected = run(&"0".repeat(64));
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("attested checksum mismatch"));
    assert_eq!(
        fs::read_to_string(root.join("bin/knapper")).unwrap(),
        "old binary"
    );
    let accepted = run(&digest);
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let version = Command::new(root.join("bin/knapper")).output().unwrap();
    assert_eq!(version.stdout, b"knapper 0.1.0\n");
}
