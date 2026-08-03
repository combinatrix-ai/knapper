//! The reference broker: provider commands, and `knapper resolve`.
//!
//! A vault says *which* provider holds a value. Only the local provider
//! config says *what runs* for that provider, and it lives outside the vault
//! on purpose: `knapper.config.md` travels with the notes, so a synced,
//! shared or cloned vault could otherwise introduce a command. This file
//! reads `$XDG_CONFIG_HOME/knapper/providers.yaml` and nothing else.
//!
//! ```yaml
//! providers:
//!   personal:
//!     command: [op, read, "op://Knapper/{locator}/value"]
//! ```
//!
//! `command` is argv, not a shell line. knapper execs it directly, so quoting
//! and word splitting never happen and there is no shell to escape from.
//! knapper is provider-agnostic: the command above is one user's choice, and
//! nothing here knows what it runs.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The one substitution a provider command gets. Everything else in argv is
/// passed through untouched.
pub const PLACEHOLDER: &str = "{locator}";

pub const CONFIG_DIR: &str = "knapper";
pub const CONFIG_FILENAME: &str = "providers.yaml";

/// A resolved value is meant to be a credential, an address, a key -- one
/// value, not a stream. Anything larger is a provider misbehaving, and
/// knapper stops reading rather than buffering it.
const MAX_OUTPUT: usize = 1024 * 1024;

/// How often a timed run checks whether the child is finished.
const POLL: Duration = Duration::from_millis(10);

/// Why a broker command failed, carrying the exit status that tells the
/// three apart. Nothing here calls `exit()`; the CLI maps these at the top
/// level, the way the rest of knapper reports usage errors.
#[derive(Debug)]
pub enum Failure {
    /// A malformed reference, a rejected argument, or unreadable config.
    Usage(String),
    /// The reference names a provider the local config does not define.
    UnknownProvider(String),
    /// The provider command would not run, failed, or returned nothing
    /// usable.
    Execution(String),
}

impl Failure {
    pub fn exit_code(&self) -> i32 {
        match self {
            Failure::Usage(_) => 2,
            Failure::UnknownProvider(_) => 3,
            Failure::Execution(_) => 4,
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Usage(message)
            | Failure::UnknownProvider(message)
            | Failure::Execution(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Failure {}

type Result<T> = std::result::Result<T, Failure>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Providers {
    #[serde(default)]
    pub providers: BTreeMap<String, Provider>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    /// argv. The first element is the program; every element has its
    /// `{locator}` occurrences replaced before the process starts.
    pub command: Vec<String>,
}

/// Where the provider config lives, given the environment.
///
/// Split from `config_path` so the rule is testable without a process-wide
/// environment. A relative `XDG_CONFIG_HOME` is ignored, as the XDG basedir
/// spec requires.
fn config_path_from(xdg: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    let base = match xdg {
        Some(value) if Path::new(&value).is_absolute() => PathBuf::from(value),
        _ => match home {
            Some(home) => PathBuf::from(home).join(".config"),
            None => {
                return Err(Failure::Usage(
                    "Cannot locate a config directory: neither XDG_CONFIG_HOME nor HOME is set"
                        .into(),
                ))
            }
        },
    };
    Ok(base.join(CONFIG_DIR).join(CONFIG_FILENAME))
}

pub fn config_path() -> Result<PathBuf> {
    config_path_from(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

/// Read the provider config. A missing file is an empty config, not an error:
/// most vaults have no providers at all.
pub fn load(path: &Path) -> Result<Providers> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Providers::default()),
        Err(err) => {
            return Err(Failure::Usage(format!(
                "Cannot read {}: {err}",
                path.display()
            )))
        }
    };
    if raw.trim().is_empty() {
        return Ok(Providers::default());
    }
    serde_yaml::from_str(&raw).map_err(|err| {
        Failure::Usage(format!(
            "Cannot parse {}: {err}. Expected:\nproviders:\n  <name>:\n    command: [prog, arg, \"...{PLACEHOLDER}...\"]",
            path.display()
        ))
    })
}

/// Write the whole config back. Everything not being changed is carried
/// through, so setting one provider never costs another; comments and key
/// order do not survive, because this file is data knapper owns.
fn save(path: &Path, providers: &Providers) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|err| Failure::Usage(format!("Cannot create {}: {err}", parent.display())))?;

    let body = serde_yaml::to_string(providers)
        .map_err(|err| Failure::Usage(format!("Cannot serialise the provider config: {err}")))?;
    std::fs::write(path, body)
        .map_err(|err| Failure::Usage(format!("Cannot write {}: {err}", path.display())))?;

    // This file decides what knapper executes, so anyone who can write it can
    // run anything as the user. Owner-only is the least this can do about it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

/// argv as an unambiguous single line. A quoted shell line would invite
/// pasting it into a shell, which is the one thing knapper never does with a
/// provider command; JSON says exactly where each argument ends and runs
/// nowhere.
fn render(argv: &[String]) -> String {
    serde_json::to_string(argv).expect("a string list serialises")
}

pub fn list(format: &str) -> Result<()> {
    let path = config_path()?;
    let configured = load(&path)?;

    if format == "json" {
        let items: Vec<serde_json::Value> = configured
            .providers
            .iter()
            .map(|(name, provider)| serde_json::json!({"name": name, "command": provider.command}))
            .collect();
        println!("{}", serde_json::to_string_pretty(&items).unwrap());
        return Ok(());
    }
    for (name, provider) in &configured.providers {
        println!("{name}  {}", render(&provider.command));
    }
    Ok(())
}

pub fn set(name: &str, command: &[String]) -> Result<()> {
    crate::refs::validate_provider(name).map_err(|err| Failure::Usage(err.to_string()))?;
    if command.is_empty() {
        return Err(Failure::Usage(
            "A provider command needs at least one argument".into(),
        ));
    }
    // Without the placeholder the command answers the same thing for every
    // reference, which means the reference was never read.
    if !command.iter().any(|arg| arg.contains(PLACEHOLDER)) {
        return Err(Failure::Usage(format!(
            "A provider command must use {PLACEHOLDER} at least once, so the reference reaches it:\n  \
             knapper provider set {name} -- op read 'op://Vault/{PLACEHOLDER}/value'"
        )));
    }

    let path = config_path()?;
    let mut configured = load(&path)?;
    configured.providers.insert(
        name.to_string(),
        Provider {
            command: command.to_vec(),
        },
    );
    save(&path, &configured)?;
    println!("Set provider {name} in {}", path.display());
    Ok(())
}

pub fn remove(name: &str) -> Result<()> {
    crate::refs::validate_provider(name).map_err(|err| Failure::Usage(err.to_string()))?;
    let path = config_path()?;
    let mut configured = load(&path)?;
    if configured.providers.remove(name).is_none() {
        return Err(Failure::UnknownProvider(format!(
            "No provider named {name:?} in {}",
            path.display()
        )));
    }
    save(&path, &configured)?;
    println!("Removed provider {name} from {}", path.display());
    Ok(())
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ResolveOptions {
    /// Seconds. There is no default: a provider may legitimately wait for a
    /// hardware key to be touched, and knapper cannot know for how long.
    pub timeout: Option<f64>,
    pub dry_run: bool,
}

fn substitute(command: &[String], locator: &str) -> Vec<String> {
    command
        .iter()
        .map(|arg| arg.replace(PLACEHOLDER, locator))
        .collect()
}

/// Strip the one line ending a command-line tool adds to its output. Any
/// other byte, including further newlines, is part of the value.
fn strip_one_newline(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text)
}

/// Read `--timeout` as a duration. Negative, zero, NaN, infinite and
/// unrepresentably large values are all the same kind of mistake, and none of
/// them reaches the clock.
fn read_timeout(seconds: Option<f64>) -> Result<Option<Duration>> {
    let Some(seconds) = seconds else {
        return Ok(None);
    };
    Duration::try_from_secs_f64(seconds)
        .ok()
        .filter(|limit| !limit.is_zero())
        .map(Some)
        .ok_or_else(|| {
            Failure::Usage(format!(
                "--timeout must be a positive number of seconds, not {seconds}"
            ))
        })
}

pub fn resolve(uri: &str, options: &ResolveOptions) -> Result<()> {
    let (provider, locator) =
        crate::refs::parse(uri).map_err(|err| Failure::Usage(err.to_string()))?;
    let timeout = read_timeout(options.timeout)?;

    let path = config_path()?;
    let configured = load(&path)?;
    let entry = configured.providers.get(&provider).ok_or_else(|| {
        Failure::UnknownProvider(format!(
            "No provider named {provider:?} in {}. Define one with:\n  \
             knapper provider set {provider} -- <command...> '{PLACEHOLDER}'",
            path.display()
        ))
    })?;
    if entry.command.is_empty() {
        return Err(Failure::Usage(format!(
            "Provider {provider:?} in {} has an empty command",
            path.display()
        )));
    }

    let argv = substitute(&entry.command, &locator);
    if options.dry_run {
        println!("{}", render(&argv));
        return Ok(());
    }

    // The value is printed and forgotten. It is not cached, not written
    // anywhere, and not scanned for further references: whatever a provider
    // returns is a value, never a reference to resolve in turn.
    let value = run(&provider, &argv, timeout)?;
    write_value(&value)
}

/// Write the value and nothing else. A value is not necessarily a line: a
/// key, token or password may end exactly where it ends, so `println!` would
/// add a byte the provider never returned.
fn write_value(value: &str) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(value.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|err| Failure::Execution(format!("Cannot write the value: {err}")))
}

fn run(provider: &str, argv: &[String], timeout: Option<Duration>) -> Result<String> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        // stdin and stderr stay attached to the terminal: a provider may need
        // to prompt for a PIN, a passphrase or a touch, and that prompt has
        // to reach whoever is sitting there. Only stdout is captured, and
        // only stdout becomes the value.
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|err| {
            Failure::Execution(format!(
                "Cannot run the command for provider {provider:?} ({:?}): {err}",
                argv[0]
            ))
        })?;

    // Read on another thread. A provider that writes more than fits in the
    // pipe buffer would block on the write while knapper blocked on the
    // exit, and neither would ever finish.
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        // One byte past the cap, so "exactly at the cap" and "over it" stay
        // distinguishable without reading an unbounded amount. Dropping the
        // pipe afterwards is what stops a runaway provider.
        let result = stdout
            .by_ref()
            .take(MAX_OUTPUT as u64 + 1)
            .read_to_end(&mut buffer)
            .map(|_| buffer);
        let _ = sender.send(result);
    });

    // One deadline bounds the whole call, including the wait for EOF. A
    // provider can exit after starting another process that inherited stdout;
    // joining the reader would otherwise let that descendant defeat
    // `--timeout` by keeping the pipe open.
    let deadline = timeout.and_then(|limit| Instant::now().checked_add(limit));
    let status = match (deadline, timeout) {
        (Some(deadline), Some(limit)) => wait_until(&mut child, deadline, limit, provider)?,
        _ => wait(&mut child)?,
    };

    let read = match deadline {
        Some(deadline) => receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|err| match err {
                std::sync::mpsc::RecvTimeoutError::Timeout => Failure::Execution(format!(
                    "The command for provider {provider:?} timed out while waiting for its output to close"
                )),
                std::sync::mpsc::RecvTimeoutError::Disconnected => {
                    Failure::Execution("The provider output reader panicked".into())
                }
            })?,
        None => receiver
            .recv()
            .map_err(|_| Failure::Execution("The provider output reader panicked".into()))?,
    };
    let bytes = read.map_err(|err| {
        Failure::Execution(format!(
            "Cannot read the output of provider {provider:?}: {err}"
        ))
    })?;

    // Size first: a provider cut off mid-write also exits badly, and "too
    // much output" is the more useful of the two reports.
    if bytes.len() > MAX_OUTPUT {
        return Err(Failure::Execution(format!(
            "Provider {provider:?} returned more than {MAX_OUTPUT} bytes"
        )));
    }
    if !status.success() {
        return Err(Failure::Execution(format!(
            "The command for provider {provider:?} failed: {status}"
        )));
    }

    let text = String::from_utf8(bytes).map_err(|_| {
        Failure::Execution(format!(
            "Provider {provider:?} returned output that is not valid UTF-8"
        ))
    })?;
    let value = strip_one_newline(&text);
    if value.is_empty() {
        return Err(Failure::Execution(format!(
            "Provider {provider:?} returned an empty value"
        )));
    }
    Ok(value.to_string())
}

fn wait(child: &mut Child) -> Result<ExitStatus> {
    child
        .wait()
        .map_err(|err| Failure::Execution(format!("Cannot wait for provider command: {err}")))
}

/// Wait for the child, giving up after `limit`.
fn wait_until(
    child: &mut Child,
    deadline: Instant,
    limit: Duration,
    provider: &str,
) -> Result<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(err) => {
                return Err(Failure::Execution(format!(
                    "Cannot wait for provider command: {err}"
                )))
            }
        }

        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            // Kill, then reap: a killed child nobody waits for stays a
            // zombie for as long as knapper runs. Dropping our end of the
            // pipe afterwards releases the reader thread.
            let _ = child.kill();
            let _ = child.wait();
            return Err(Failure::Execution(format!(
                "The command for provider {provider:?} timed out after {}s",
                limit.as_secs_f64()
            )));
        }
        std::thread::sleep(POLL.min(left));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_placeholder_in_every_argument_is_replaced() {
        assert_eq!(
            substitute(
                &strings(&["op", "read", "op://V/{locator}/value", "--out={locator}"]),
                "a/b"
            ),
            strings(&["op", "read", "op://V/a/b/value", "--out=a/b"])
        );
        // Twice in one argument, and none at all, both behave.
        assert_eq!(
            substitute(&strings(&["{locator}-{locator}", "fixed"]), "x"),
            strings(&["x-x", "fixed"])
        );
    }

    /// A value can be several lines, and only the terminator a shell tool
    /// adds comes off.
    #[test]
    fn exactly_one_trailing_newline_is_stripped() {
        for (raw, expected) in [
            ("value\n", "value"),
            ("value\r\n", "value"),
            ("value", "value"),
            ("value\n\n", "value\n"),
            ("a\nb\n", "a\nb"),
            ("a\nb", "a\nb"),
            ("  spaced  \n", "  spaced  "),
            ("\n", ""),
            ("", ""),
        ] {
            assert_eq!(strip_one_newline(raw), expected, "input: {raw:?}");
        }
    }

    /// A timeout reaches a clock, so everything a clock cannot hold is
    /// refused before it gets there rather than panicking inside it.
    #[test]
    fn a_timeout_is_read_as_a_positive_duration_or_not_at_all() {
        assert_eq!(read_timeout(None).unwrap(), None);
        assert_eq!(
            read_timeout(Some(0.25)).unwrap(),
            Some(Duration::from_millis(250))
        );
        for rejected in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e30] {
            let err = read_timeout(Some(rejected)).unwrap_err();
            assert_eq!(err.exit_code(), 2, "should reject {rejected}");
            assert!(err.to_string().contains("--timeout"), "{err}");
        }
    }

    #[test]
    fn argv_is_rendered_unambiguously() {
        assert_eq!(
            render(&strings(&["op", "read", "op://V/a b/value"])),
            r#"["op","read","op://V/a b/value"]"#
        );
    }

    #[test]
    fn the_config_path_prefers_xdg_and_ignores_a_relative_one() {
        let xdg = |p: &str| Some(OsString::from(p));
        assert_eq!(
            config_path_from(xdg("/xdg"), xdg("/home/u")).unwrap(),
            PathBuf::from("/xdg/knapper/providers.yaml")
        );
        assert_eq!(
            config_path_from(None, xdg("/home/u")).unwrap(),
            PathBuf::from("/home/u/.config/knapper/providers.yaml")
        );
        // Relative values are ignored, as the XDG basedir spec says.
        assert_eq!(
            config_path_from(xdg("relative"), xdg("/home/u")).unwrap(),
            PathBuf::from("/home/u/.config/knapper/providers.yaml")
        );
        assert!(config_path_from(None, None).is_err());
    }

    #[test]
    fn a_missing_or_empty_config_holds_no_providers() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&dir.path().join("providers.yaml"))
            .unwrap()
            .providers
            .is_empty());

        let empty = dir.path().join("empty.yaml");
        std::fs::write(&empty, "\n").unwrap();
        assert!(load(&empty).unwrap().providers.is_empty());
    }

    #[test]
    fn saving_one_provider_carries_the_others_through() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("providers.yaml");

        let mut configured = Providers::default();
        configured.providers.insert(
            "work".into(),
            Provider {
                command: strings(&["work-tool", "{locator}"]),
            },
        );
        save(&path, &configured).unwrap();

        let mut reloaded = load(&path).unwrap();
        reloaded.providers.insert(
            "personal".into(),
            Provider {
                command: strings(&["op", "read", "op://Knapper/{locator}/value"]),
            },
        );
        save(&path, &reloaded).unwrap();

        let final_state = load(&path).unwrap();
        assert_eq!(final_state.providers.len(), 2);
        assert_eq!(
            final_state.providers["work"].command,
            strings(&["work-tool", "{locator}"])
        );
        assert_eq!(
            final_state.providers["personal"].command,
            strings(&["op", "read", "op://Knapper/{locator}/value"])
        );
    }

    #[test]
    fn a_malformed_config_is_a_usage_failure_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("providers.yaml");
        std::fs::write(&path, "providers:\n  personal:\n    command: \"op read\"\n").unwrap();

        let err = load(&path).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("providers.yaml"), "{err}");
    }

    #[test]
    fn the_three_failures_have_distinct_exit_codes() {
        assert_eq!(Failure::Usage(String::new()).exit_code(), 2);
        assert_eq!(Failure::UnknownProvider(String::new()).exit_code(), 3);
        assert_eq!(Failure::Execution(String::new()).exit_code(), 4);
    }
}
