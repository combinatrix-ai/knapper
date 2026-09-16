//! Opportunistic six-hour update checks. No note content leaves this process.
use std::fs::{self, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .map(|p| p.join("knapper"))
}

fn due(path: &Path, now: SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| now.duration_since(t).ok())
        .is_none_or(|age| age >= INTERVAL)
}

pub fn notify_and_schedule() {
    if !std::io::stderr().is_terminal()
        || std::env::var_os("CI").is_some()
        || std::env::var_os("KNAPPER_NO_UPDATE_CHECK").is_some()
    {
        return;
    }
    let Some(dir) = cache_dir() else {
        return;
    };
    if let Ok(latest) = fs::read_to_string(dir.join("latest-version")) {
        if crate::update::version_is_newer(latest.trim(), env!("CARGO_PKG_VERSION")) {
            eprintln!(
                "knapper {} is available; run `knapper self-update`.",
                latest.trim()
            );
        }
    }
    if due(&dir.join("checked"), SystemTime::now()) {
        if let Ok(exe) = std::env::current_exe() {
            let mut command = Command::new(exe);
            command
                .arg("update-check-internal")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            let _ = command.spawn();
        }
    }
}

pub fn refresh() -> anyhow::Result<()> {
    let Some(dir) = cache_dir() else {
        return Ok(());
    };
    fs::create_dir_all(&dir)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("check.lock"))?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // The descriptor remains owned by `lock` until the check finishes.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Ok(());
        }
    }
    if !due(&dir.join("checked"), SystemTime::now()) {
        return Ok(());
    }
    fs::write(dir.join("checked"), b"")?;
    // A failed check retains the previous successfully discovered version.
    let latest = crate::update::resolve_latest_version()?;
    let mut staged = tempfile::NamedTempFile::new_in(&dir)?;
    staged.write_all(latest.as_bytes())?;
    staged.persist(dir.join("latest-version"))?;
    drop(lock);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_marker_checks_now_and_recent_marker_waits_six_hours() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("checked");
        assert!(due(&path, SystemTime::now()));
        fs::write(&path, b"").unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(!due(&path, modified + Duration::from_secs(60)));
        assert!(due(&path, modified + INTERVAL));
    }
}
