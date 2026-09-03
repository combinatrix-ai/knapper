//! End-to-end proof for the privacy boundary of the optional Chrome bridge.
//!
//! A fake provider value is allowed on the Chrome-owned Native Messaging
//! frame, but it must never appear in the Codex-facing client's stdout or
//! stderr. The real host and client binaries are used.

#![cfg(unix)]

#[path = "../src/chrome_bridge.rs"]
mod bridge;

use std::io::BufReader;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use bridge::{read_json, write_json, NativeMessage};

#[test]
fn resolved_value_crosses_only_the_native_messaging_pipe() {
    const VALUE: &str = "fake-private-value-for-transport-test";
    const ORIGIN: &str = "https://example.test";
    const URL: &str = "https://example.test/form";

    let home = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir_in("/tmp").unwrap();
    let resolver = home.path().join("fake-knapper");
    std::fs::write(&resolver, format!("#!/bin/sh\nprintf '%s' '{VALUE}'\n")).unwrap();
    std::fs::set_permissions(&resolver, std::fs::Permissions::from_mode(0o700)).unwrap();

    let mut host = Command::new(env!("CARGO_BIN_EXE_knapper-chrome-host"))
        .env("XDG_CONFIG_HOME", home.path())
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("KNAPPER_BINARY", &resolver)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut native_in = host.stdin.take().unwrap();
    let mut native_out = BufReader::new(host.stdout.take().unwrap());

    write_json(
        &mut native_in,
        &NativeMessage::Arm {
            tab_id: 7,
            url: URL.into(),
            origin: ORIGIN.into(),
        },
    )
    .unwrap();
    let armed = match read_json::<_, NativeMessage>(&mut native_out) {
        Ok(message) => message,
        Err(error) => {
            drop(native_in);
            let output = host.wait_with_output().unwrap();
            panic!(
                "host closed before arming: {error}; stderr={}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    };
    assert_eq!(armed, NativeMessage::Armed);

    let client = Command::new(env!("CARGO_BIN_EXE_knapper-chrome-client"))
        .args([
            "knapper://personal/address.home",
            "--expected-origin",
            ORIGIN,
            "--timeout",
            "5",
        ])
        .env("XDG_CONFIG_HOME", home.path())
        .env("XDG_RUNTIME_DIR", runtime.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let prepared: NativeMessage = read_json(&mut native_out).unwrap();
    let request_id = match prepared {
        NativeMessage::PrepareFill {
            request_id,
            reference,
            expected_origin,
            expected_url,
            ..
        } => {
            assert_eq!(reference, "knapper://personal/address.home");
            assert_eq!(expected_origin, ORIGIN);
            assert_eq!(expected_url, URL);
            request_id
        }
        message => panic!("expected prepare_fill, got {message:?}"),
    };

    write_json(
        &mut native_in,
        &NativeMessage::TargetReady {
            request_id: request_id.clone(),
        },
    )
    .unwrap();
    let fill: NativeMessage = read_json(&mut native_out).unwrap();
    assert_eq!(
        fill,
        NativeMessage::Fill {
            request_id: request_id.clone(),
            value: VALUE.into(),
        }
    );

    // The continuous-selection extension announces the next selection before
    // acknowledging this fill. The host must retain the pending request and
    // expose the new safe mode while it waits for Filled.
    write_json(
        &mut native_in,
        &NativeMessage::Selecting {
            tab_id: 7,
            url: URL.into(),
            origin: ORIGIN.into(),
        },
    )
    .unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_knapper-chrome-client"))
        .arg("status")
        .env("XDG_CONFIG_HOME", home.path())
        .env("XDG_RUNTIME_DIR", runtime.path())
        .output()
        .unwrap();
    assert!(status.status.success());
    let status_stdout = String::from_utf8(status.stdout).unwrap();
    assert!(status_stdout.contains(r#""mode":"selecting""#));
    assert!(!status_stdout.contains(VALUE));
    assert!(!status_stdout.contains("/form"));

    write_json(
        &mut native_in,
        &NativeMessage::Filled {
            request_id: request_id.clone(),
        },
    )
    .unwrap();

    let output = client.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains(r#""status":"filled""#));
    assert!(stdout.contains(ORIGIN));
    assert!(!stdout.contains(VALUE));
    assert!(!stdout.contains("value"));
    assert!(!stderr.contains(VALUE));

    drop(native_in);
    let output = host.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(VALUE));
}

#[test]
fn status_reports_safe_mode_without_reference_or_page_url() {
    const ORIGIN: &str = "https://example.test";
    const URL: &str = "https://example.test/form";

    let home = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir_in("/tmp").unwrap();
    let mut host = Command::new(env!("CARGO_BIN_EXE_knapper-chrome-host"))
        .env("XDG_CONFIG_HOME", home.path())
        .env("XDG_RUNTIME_DIR", runtime.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut native_in = host.stdin.take().unwrap();
    let _native_out = BufReader::new(host.stdout.take().unwrap());

    let socket = runtime.path().join("knapper").join(bridge::SOCKET_FILENAME);
    // CI hosts can take longer than half a second to schedule the freshly
    // spawned native host, especially while the test binary is running its
    // other process-based cases in parallel.
    for _ in 0..1000 {
        if socket.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(socket.exists(), "host did not create its runtime socket");

    write_json(
        &mut native_in,
        &NativeMessage::Selecting {
            tab_id: 7,
            url: URL.into(),
            origin: ORIGIN.into(),
        },
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_knapper-chrome-client"))
        .arg("status")
        .env("XDG_CONFIG_HOME", home.path())
        .env("XDG_RUNTIME_DIR", runtime.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.contains(r#""status":"ok""#));
    assert!(stdout.contains(r#""mode":"selecting""#));
    assert!(stdout.contains(ORIGIN));
    assert!(!stdout.contains("/form"));
    assert!(!stdout.contains("reference"));
    assert!(!stdout.contains("secret"));
    assert!(stderr.is_empty());

    drop(native_in);
    let output = host.wait_with_output().unwrap();
    assert!(output.status.success());
}

#[test]
fn status_without_socket_emits_one_json_error_line() {
    let home = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir_in("/tmp").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_knapper-chrome-client"))
        .arg("status")
        .env("XDG_CONFIG_HOME", home.path())
        .env("XDG_RUNTIME_DIR", runtime.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.contains(r#""status":"error""#));
    assert!(stdout.contains(r#""code":"not_connected""#));
    assert!(!stdout.contains("reference"));
    assert!(!stdout.contains("secret"));
}
