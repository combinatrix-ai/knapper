//! The agent skill, embedded in the binary.
//!
//! Shipping it inside the executable rather than as a file beside it keeps
//! the "one binary, nothing to install alongside it" claim true: an agent
//! host can be handed the skill by running `knapper skill`, and an installer
//! can register it without fetching anything more.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const TEXT: &str = include_str!("../assets/knapper-skill.md");

/// Where each agent host looks for skills.
///
/// Only hosts that appear to be present are written to, so installing
/// knapper does not create a `.codex` directory on a machine that has never
/// run Codex.
fn skill_targets() -> Vec<(&'static str, PathBuf)> {
    let mut targets = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);

    let codex = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".codex")));
    if let Some(dir) = codex {
        if dir.is_dir() {
            targets.push(("codex", dir.join("skills/knapper/SKILL.md")));
        }
    }

    if let Some(dir) = home.map(|h| h.join(".claude")) {
        if dir.is_dir() {
            targets.push(("claude", dir.join("skills/knapper/SKILL.md")));
        }
    }

    targets
}

fn write_skill(path: &Path) -> Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|current| current == TEXT) {
        return Ok(false);
    }
    let parent = path.parent().context("skill path has no directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("could not create {}", parent.display()))?;

    // Never discard a skill somebody may have edited by hand.
    if path.exists() {
        let stamp = chrono::Local::now().format("%Y%m%d%H%M%S");
        let backup = path.with_extension(format!("md.backup.{stamp}"));
        std::fs::rename(path, &backup)
            .with_context(|| format!("could not preserve {}", path.display()))?;
        println!("Preserved the existing skill: {}", backup.display());
    }

    std::fs::write(path, TEXT).with_context(|| format!("could not write {}", path.display()))?;
    Ok(true)
}

/// Register the skill with every agent host on this machine.
///
/// Called after a successful self-update so the skill cannot drift behind the
/// binary that documents itself.
pub fn register() {
    for (host, path) in skill_targets() {
        match write_skill(&path) {
            Ok(true) => println!(
                "Registered the knapper skill for {host}: {}",
                path.display()
            ),
            Ok(false) => {}
            // A skill that cannot be written is worth saying, but never worth
            // failing an update over.
            Err(err) => eprintln!("Warning: could not register the {host} skill: {err}"),
        }
    }
}

pub fn run(install: bool) -> Result<()> {
    if !install {
        print!("{TEXT}");
        return Ok(());
    }

    let targets = skill_targets();
    if targets.is_empty() {
        println!("No agent host found. Looked for ~/.codex and ~/.claude.");
        println!("Write the skill yourself with: knapper skill > path/to/SKILL.md");
        return Ok(());
    }
    for (host, path) in targets {
        if write_skill(&path)? {
            println!(
                "Registered the knapper skill for {host}: {}",
                path.display()
            );
        } else {
            println!("Already current for {host}: {}", path.display());
        }
    }
    Ok(())
}
