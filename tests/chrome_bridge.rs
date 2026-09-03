//! End-to-end proof for the privacy boundary of the optional Chrome bridge.
//!
//! A fake provider value is allowed on the Chrome-owned Native Messaging
//! frame, but it must never appear in the Codex-facing client's stdout or
//! stderr. The real host and client binaries are used.

#![cfg(unix)]

#[path = "../src/chrome_bridge.rs"]
mod bridge;

use std::io::{BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use bridge::{
    read_json, write_json, BrowserTab, FormActionResult, FormControl, FormDescription,
    FormSnapshot, NativeFormAction, NativeMessage,
};

fn wait_for_mode(home: &std::path::Path, runtime: &std::path::Path, expected: &str) {
    for _ in 0..100 {
        let output = Command::new(env!("CARGO_BIN_EXE_knapper-chrome-client"))
            .arg("status")
            .env("XDG_CONFIG_HOME", home)
            .env("XDG_RUNTIME_DIR", runtime)
            .output()
            .unwrap();
        if output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains(&format!(r#""mode":"{expected}""#))
        {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("host did not enter {expected} mode");
}

fn spawn_api_client(
    home: &std::path::Path,
    runtime: &std::path::Path,
    request: &str,
) -> std::process::Child {
    let mut client = Command::new(env!("CARGO_BIN_EXE_knapper-chrome-client"))
        .args(["api", "--timeout", "5"])
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_RUNTIME_DIR", runtime)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    client
        .stdin
        .take()
        .unwrap()
        .write_all(request.as_bytes())
        .unwrap();
    client
}

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

#[test]
fn all_mode_lists_tabs_without_an_active_tab_selection() {
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
    let mut native_out = BufReader::new(host.stdout.take().unwrap());

    write_json(&mut native_in, &NativeMessage::Mode { mode: "all".into() }).unwrap();
    wait_for_mode(home.path(), runtime.path(), "all");

    let client = spawn_api_client(
        home.path(),
        runtime.path(),
        r#"{"op":"tabs_list","origin":"https://example.test"}"#,
    );
    let request_id = match read_json::<_, NativeMessage>(&mut native_out).unwrap() {
        NativeMessage::TabsList { request_id, origin } => {
            assert_eq!(origin.as_deref(), Some(ORIGIN));
            request_id
        }
        message => panic!("expected tabs_list, got {message:?}"),
    };
    write_json(
        &mut native_in,
        &NativeMessage::TabsListed {
            request_id,
            tabs: vec![BrowserTab {
                tab_id: 41,
                url: URL.into(),
                origin: ORIGIN.into(),
                active: false,
            }],
        },
    )
    .unwrap();

    let output = client.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(r#""kind":"tabs_list""#));
    assert!(stdout.contains(r#""tab_id":41"#));
    assert!(stdout.contains(r#""active":false"#));
    assert!(String::from_utf8(output.stderr).unwrap().is_empty());

    drop(native_in);
    assert!(host.wait_with_output().unwrap().status.success());
}

#[test]
fn opaque_snapshot_values_are_rejected_without_echoing_them() {
    const SECRET: &str = "must-not-cross-the-client-socket";
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
    let mut native_out = BufReader::new(host.stdout.take().unwrap());
    write_json(&mut native_in, &NativeMessage::Mode { mode: "all".into() }).unwrap();
    wait_for_mode(home.path(), runtime.path(), "all");

    let client = spawn_api_client(
        home.path(),
        runtime.path(),
        r#"{"op":"form_snapshot","tab_id":41}"#,
    );
    let request_id = match read_json::<_, NativeMessage>(&mut native_out).unwrap() {
        NativeMessage::FormSnapshot { request_id, tab_id } => {
            assert_eq!(tab_id, 41);
            request_id
        }
        message => panic!("expected form_snapshot, got {message:?}"),
    };
    write_json(
        &mut native_in,
        &NativeMessage::FormSnapshotted {
            request_id,
            snapshot: FormSnapshot {
                document_id: "document_1".into(),
                tab_id: 41,
                url: URL.into(),
                origin: ORIGIN.into(),
                forms: vec![FormDescription {
                    form_id: "form_1".into(),
                    method: Some("post".into()),
                    action: Some(URL.into()),
                    controls: vec![FormControl {
                        target_id: "target_1".into(),
                        form_id: Some("form_1".into()),
                        tag: "input".into(),
                        kind: "text".into(),
                        control_type: Some("text".into()),
                        id_attr: None,
                        name: Some("address".into()),
                        label: Some("Address".into()),
                        autocomplete: None,
                        required: true,
                        disabled: false,
                        read_only: false,
                        checked: None,
                        current_value: Some(SECRET.into()),
                        value_opaque: true,
                        options: Vec::new(),
                    }],
                }],
            },
        },
    )
    .unwrap();

    let output = client.wait_with_output().unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains(r#""code":"protocol_error""#));
    assert!(!stdout.contains(SECRET));
    assert!(!stderr.contains(SECRET));

    drop(native_in);
    let output = host.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(SECRET));
}

#[test]
fn snapshot_and_explicit_submit_are_separate_api_requests() {
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
    let mut native_out = BufReader::new(host.stdout.take().unwrap());
    write_json(&mut native_in, &NativeMessage::Mode { mode: "all".into() }).unwrap();
    wait_for_mode(home.path(), runtime.path(), "all");

    let snapshot_client = spawn_api_client(
        home.path(),
        runtime.path(),
        r#"{"op":"form_snapshot","tab_id":41}"#,
    );
    let snapshot_request_id = match read_json::<_, NativeMessage>(&mut native_out).unwrap() {
        NativeMessage::FormSnapshot { request_id, tab_id } => {
            assert_eq!(tab_id, 41);
            request_id
        }
        message => panic!("expected form_snapshot, got {message:?}"),
    };
    write_json(
        &mut native_in,
        &NativeMessage::FormSnapshotted {
            request_id: snapshot_request_id,
            snapshot: FormSnapshot {
                document_id: "document_1".into(),
                tab_id: 41,
                url: URL.into(),
                origin: ORIGIN.into(),
                forms: vec![FormDescription {
                    form_id: "form_1".into(),
                    method: Some("post".into()),
                    action: Some(URL.into()),
                    controls: Vec::new(),
                }],
            },
        },
    )
    .unwrap();
    let snapshot_output = snapshot_client.wait_with_output().unwrap();
    assert!(snapshot_output.status.success());
    assert!(String::from_utf8(snapshot_output.stdout)
        .unwrap()
        .contains(r#""kind":"form_snapshot""#));

    let submit_client = spawn_api_client(
        home.path(),
        runtime.path(),
        r#"{"op":"form_submit","tab_id":41,"document_id":"document_1","form_id":"form_1"}"#,
    );
    let submit_request_id = match read_json::<_, NativeMessage>(&mut native_out).unwrap() {
        NativeMessage::FormSubmit {
            request_id,
            tab_id,
            document_id,
            form_id,
        } => {
            assert_eq!(tab_id, 41);
            assert_eq!(document_id, "document_1");
            assert_eq!(form_id, "form_1");
            request_id
        }
        message => panic!("expected form_submit, got {message:?}"),
    };
    write_json(
        &mut native_in,
        &NativeMessage::FormSubmitted {
            request_id: submit_request_id,
            document_id: "document_1".into(),
            status: "submitted".into(),
        },
    )
    .unwrap();
    let submit_output = submit_client.wait_with_output().unwrap();
    assert!(submit_output.status.success());
    let stdout = String::from_utf8(submit_output.stdout).unwrap();
    assert!(stdout.contains(r#""kind":"form_submit""#));
    assert!(stdout.contains(r#""state":"submitted""#));

    drop(native_in);
    assert!(host.wait_with_output().unwrap().status.success());
}

#[test]
fn set_from_is_resolved_only_onto_the_native_pipe() {
    const VALUE: &str = "fake-private-value-for-all-mode";
    const REFERENCE: &str = "knapper://personal/address.home";

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
    write_json(&mut native_in, &NativeMessage::Mode { mode: "all".into() }).unwrap();
    wait_for_mode(home.path(), runtime.path(), "all");

    let client = spawn_api_client(
        home.path(),
        runtime.path(),
        r#"{"op":"form_perform","tab_id":41,"document_id":"document_1","actions":[{"op":"set_from","target_id":"target_1","reference":"knapper://personal/address.home"},{"op":"set_checked","target_id":"target_2","checked":true}]}"#,
    );
    let request_id = match read_json::<_, NativeMessage>(&mut native_out).unwrap() {
        NativeMessage::FormPerform {
            request_id,
            tab_id,
            document_id,
            actions,
        } => {
            assert_eq!(tab_id, 41);
            assert_eq!(document_id, "document_1");
            assert_eq!(
                actions,
                vec![
                    NativeFormAction::SetValue {
                        target_id: "target_1".into(),
                        value: VALUE.into(),
                        opaque: true,
                    },
                    NativeFormAction::SetChecked {
                        target_id: "target_2".into(),
                        checked: true,
                    },
                ]
            );
            request_id
        }
        message => panic!("expected form_perform, got {message:?}"),
    };
    write_json(
        &mut native_in,
        &NativeMessage::FormPerformed {
            request_id,
            document_id: "document_1".into(),
            results: vec![
                FormActionResult {
                    target_id: "target_1".into(),
                    op: "set_value".into(),
                    status: "verified".into(),
                    code: None,
                    value_returned: Some(false),
                },
                FormActionResult {
                    target_id: "target_2".into(),
                    op: "set_checked".into(),
                    status: "verified".into(),
                    code: None,
                    value_returned: None,
                },
            ],
        },
    )
    .unwrap();

    let output = client.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains(r#""kind":"form_perform""#));
    assert!(stdout.contains(r#""status":"verified""#));
    assert!(!stdout.contains(VALUE));
    assert!(!stdout.contains(REFERENCE));
    assert!(!stderr.contains(VALUE));
    assert!(!stderr.contains(REFERENCE));

    drop(native_in);
    let output = host.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(VALUE));
}

#[test]
fn generic_click_is_not_part_of_the_local_api() {
    let home = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir_in("/tmp").unwrap();
    let mut client = Command::new(env!("CARGO_BIN_EXE_knapper-chrome-client"))
        .arg("api")
        .env("XDG_CONFIG_HOME", home.path())
        .env("XDG_RUNTIME_DIR", runtime.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    client
        .stdin
        .take()
        .unwrap()
        .write_all(
            br#"{"op":"form_perform","tab_id":41,"document_id":"document_1","actions":[{"op":"click","target_id":"target_1"}]}"#,
        )
        .unwrap();

    let output = client.wait_with_output().unwrap();
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.contains(r#""code":"invalid_request""#));
    assert!(!stdout.contains("target_1"));
}
