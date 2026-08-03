//! The broker end to end: what a provider command sees, and what knapper
//! does with what it writes back.
//!
//! These are the cases `tests/contract/cases.yaml` cannot express, because
//! they need exact bytes, a clock, or a second run: invalid UTF-8, output
//! past the cap, a timeout, the config-home rule, and the absence of a cache.
//!
//! Each case drives the real binary against a config home of its own, so
//! nothing here reads -- or writes -- the provider config of whoever is
//! running the suite.
//!
//! The provider commands below are mostly `sh -c ...`, which is how a fixture
//! produces exact bytes. knapper itself never invokes a shell: it execs
//! argv[0] directly, and one of the cases pins that.

#![cfg(unix)]

use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// Write a local provider config declaring one provider named `personal`.
fn write_providers(config_home: &Path, command: &[&str]) {
    let dir = config_home.join("knapper");
    std::fs::create_dir_all(&dir).unwrap();
    // YAML is a superset of JSON, so a JSON array is a flow sequence -- and
    // serialising it is the one way to be sure the argv in the file is the
    // argv this test meant.
    let argv = serde_json::to_string(command).unwrap();
    std::fs::write(
        dir.join("providers.yaml"),
        format!("providers:\n  personal:\n    command: {argv}\n"),
    )
    .unwrap();
}

fn broker_running(command: &[&str]) -> TempDir {
    let home = tempfile::tempdir().unwrap();
    write_providers(home.path(), command);
    home
}

fn knapper(config_home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_knapper"))
        .args(args)
        .current_dir(config_home)
        .env("XDG_CONFIG_HOME", config_home)
        .output()
        .expect("knapper runs")
}

fn resolve(config_home: &Path, args: &[&str]) -> Output {
    let mut all = vec!["resolve", "knapper://personal/address.home"];
    all.extend_from_slice(args);
    knapper(config_home, &all)
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_failed(output: &Output, expected: i32, needle: &str) {
    assert_eq!(
        code(output),
        expected,
        "expected exit {expected}, stderr was: {}",
        stderr(output)
    );
    assert!(
        stderr(output).contains(needle),
        "stderr lacks {needle:?}: {}",
        stderr(output)
    );
}

/// A value can be several lines, and every byte of it except the terminator
/// the provider's own tooling added is the value.
#[test]
fn a_multiline_value_survives_intact() {
    let home = broker_running(&["printf", "%s\n", "line one\nline two\n\nline four"]);
    let output = resolve(home.path(), &[]);

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(output.stdout, b"line one\nline two\n\nline four");
}

/// A provider that ends its output with CRLF gets exactly that one ending
/// removed -- not every \r, and not a second line.
#[test]
fn one_crlf_comes_off_and_no_more() {
    let home = broker_running(&["printf", "a\r\nb\r\n"]);
    let output = resolve(home.path(), &[]);

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(output.stdout, b"a\r\nb");
}

/// A value is text. Bytes that are not are a provider knapper cannot speak
/// for, so it says so instead of printing something lossy.
#[test]
fn output_that_is_not_utf8_is_a_failure() {
    let home = broker_running(&["sh", "-c", r"printf '\377\376'"]);
    let output = resolve(home.path(), &[]);

    assert_failed(&output, 4, "not valid UTF-8");
    assert!(output.stdout.is_empty());
}

/// A resolved value is one value, not a stream. Past the cap knapper stops
/// reading rather than buffering whatever is still coming.
#[test]
fn output_past_the_cap_is_refused_rather_than_buffered() {
    let home = broker_running(&["sh", "-c", "yes aaaaaaaaaaaaaaaa | head -c 1200000"]);
    let output = resolve(home.path(), &[]);

    assert_failed(&output, 4, "more than");
    assert!(output.stdout.is_empty());
}

/// There is no default timeout, because a provider may legitimately wait for
/// a hardware key to be touched or a passphrase to be typed.
#[test]
fn a_slow_provider_is_waited_for_when_no_timeout_is_given() {
    let home = broker_running(&["sh", "-c", "sleep 1; printf '%s\\n' '{locator}'"]);
    let output = resolve(home.path(), &[]);

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(output.stdout, b"address.home");
}

/// ...and when one is given, it ends the child rather than waiting it out.
#[test]
fn a_timeout_ends_the_child_and_reports_it() {
    let home = broker_running(&["sh", "-c", "sleep 30"]);

    let started = Instant::now();
    let output = resolve(home.path(), &["--timeout", "0.3"]);
    let elapsed = started.elapsed();

    assert_failed(&output, 4, "timed out");
    assert!(
        elapsed < Duration::from_secs(10),
        "waited {elapsed:?}, which is the sleep rather than the timeout"
    );
}

/// A deadline bounds the whole resolve, not just the direct child. The shell
/// exits immediately, but its background child keeps the inherited stdout
/// pipe open; waiting for EOF must not defeat `--timeout`.
#[test]
fn a_timeout_bounds_output_held_open_by_a_descendant() {
    // Close the descendant's stderr so the outer test harness is measuring
    // knapper's provider-stdout deadline, not waiting on an unrelated copy of
    // the harness's own captured stderr descriptor.
    let home = broker_running(&["sh", "-c", "printf hello; sleep 5 2>/dev/null & :"]);

    let started = Instant::now();
    let output = resolve(home.path(), &["--timeout", "0.2"]);
    let elapsed = started.elapsed();

    assert_failed(&output, 4, "timed out");
    assert!(
        elapsed < Duration::from_secs(3),
        "waited {elapsed:?} for the descendant rather than the timeout"
    );
}

/// A value that is not a line does not become one when knapper emits it.
#[test]
fn a_value_with_no_trailing_newline_gains_nothing() {
    let home = broker_running(&["printf", "%s", "no-trailing-newline"]);
    let output = resolve(home.path(), &[]);

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(output.stdout, b"no-trailing-newline");
}

/// When two newlines are returned, one is the provider's terminator and one
/// belongs to the value, so exactly one remains.
#[test]
fn a_value_that_ends_in_a_newline_keeps_it() {
    let home = broker_running(&["printf", "a\n\n"]);
    let output = resolve(home.path(), &[]);

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(output.stdout, b"a\n");
}

/// argv is argv. A shell would read the ";" and the "$(...)" below; knapper
/// hands them to the program as one literal argument, because there is no
/// shell in the path at all.
#[test]
fn a_provider_command_is_never_a_shell_line() {
    let home = broker_running(&["printf", "%s\n", "a; echo pwned $(id) `id` | tee"]);
    let output = resolve(home.path(), &[]);

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(output.stdout, b"a; echo pwned $(id) `id` | tee");
}

/// A provider is somebody else's tool: it needs the environment it was
/// started in to find its own config, agent socket and credentials.
#[test]
fn the_provider_inherits_the_environment() {
    let home = broker_running(&["sh", "-c", "printf '%s\\n' \"$KNAPPER_TEST_MARKER\""]);
    let output = Command::new(env!("CARGO_BIN_EXE_knapper"))
        .args(["resolve", "knapper://personal/address.home"])
        .current_dir(home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env("KNAPPER_TEST_MARKER", "inherited")
        .output()
        .expect("knapper runs");

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(output.stdout, b"inherited");
}

/// Whatever a provider returns is a value. It is not markdown, it is not a
/// reference, and it is not looked at again.
#[test]
fn a_returned_reference_is_printed_rather_than_followed() {
    let home = broker_running(&["printf", "%s\n", "knapper://personal/somewhere.else"]);
    let output = resolve(home.path(), &[]);

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(output.stdout, b"knapper://personal/somewhere.else");
}

/// No cache and nothing persisted: the second resolve runs the provider
/// again, and neither run leaves the value anywhere.
#[test]
fn resolving_twice_asks_the_provider_twice_and_stores_nothing() {
    let home = tempfile::tempdir().unwrap();
    let ledger = home.path().join("ran");
    write_providers(
        home.path(),
        &[
            "sh",
            "-c",
            &format!(
                "printf x >> {}; printf '%s\\n' 'the-value-{{locator}}'",
                ledger.display()
            ),
        ],
    );

    for _ in 0..2 {
        let output = resolve(home.path(), &[]);
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        assert_eq!(output.stdout, b"the-value-address.home");
    }

    assert_eq!(
        std::fs::read_to_string(&ledger).unwrap(),
        "xx",
        "the provider should have run once per resolve"
    );

    // The only thing under the config home is still the config itself.
    let mut left: Vec<String> = std::fs::read_dir(home.path().join("knapper"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, ["providers.yaml"]);

    // The template is still a template: the locator that was resolved -- and
    // so the value built from it -- was never written back.
    let config = std::fs::read_to_string(home.path().join("knapper/providers.yaml")).unwrap();
    assert!(config.contains("{locator}"));
    assert!(
        !config.contains("address.home"),
        "the config holds what was resolved"
    );
}

// ------------------------------------------------------------ config home --

/// With no XDG_CONFIG_HOME, the config is where the XDG basedir spec says it
/// defaults to.
#[test]
fn the_config_falls_back_to_home_dot_config() {
    let home = tempfile::tempdir().unwrap();
    write_providers(
        &home.path().join(".config"),
        &["printf", "%s\n", "{locator}"],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_knapper"))
        .args(["resolve", "knapper://personal/address.home"])
        .current_dir(home.path())
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", home.path())
        .output()
        .expect("knapper runs");

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(output.stdout, b"address.home");
}

/// Where both are set, XDG_CONFIG_HOME is the one that counts -- including
/// for the file `provider set` writes.
#[test]
fn xdg_config_home_wins_over_home() {
    let xdg = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    write_providers(&home.path().join(".config"), &["false", "{locator}"]);

    let output = Command::new(env!("CARGO_BIN_EXE_knapper"))
        .args([
            "provider",
            "set",
            "personal",
            "--",
            "printf",
            "%s\n",
            "{locator}",
        ])
        .current_dir(xdg.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .env("HOME", home.path())
        .output()
        .expect("knapper runs");
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    assert!(xdg.path().join("knapper/providers.yaml").exists());
    assert!(
        std::fs::read_to_string(home.path().join(".config/knapper/providers.yaml"))
            .unwrap()
            .contains("false")
    );
}

/// The provider config is what knapper executes, so it is not a file to
/// leave readable by everyone on the machine.
#[test]
fn a_written_config_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir().unwrap();
    let output = knapper(
        home.path(),
        &["provider", "set", "personal", "--", "printf", "{locator}"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let mode = std::fs::metadata(home.path().join("knapper/providers.yaml"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
}
