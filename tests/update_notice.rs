#![cfg(unix)]
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn background_checks_cache_results_throttle_and_preserve_notice_on_failure() {
    let temp = tempdir().unwrap();
    let curl = temp.path().join("curl");
    fs::write(
        &curl,
        "#!/bin/sh\nprintf 'https://github.com/combinatrix-ai/knapper/releases/tag/v0.2.0'\n",
    )
    .unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_knapper"))
            .arg("update-check-internal")
            .env("XDG_CACHE_HOME", temp.path().join("cache"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    temp.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .output()
            .unwrap()
    };
    assert!(run().status.success());
    let dir = temp.path().join("cache/knapper");
    assert_eq!(
        fs::read_to_string(dir.join("latest-version")).unwrap(),
        "0.2.0"
    );
    fs::write(&curl, "#!/bin/sh\nexit 1\n").unwrap();
    assert!(
        run().status.success(),
        "recent check must not hit transport"
    );
    fs::remove_file(dir.join("checked")).unwrap();
    assert!(!run().status.success());
    assert_eq!(
        fs::read_to_string(dir.join("latest-version")).unwrap(),
        "0.2.0"
    );
}
