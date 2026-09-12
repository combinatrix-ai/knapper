use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::vault::{relative_path, resolve_path, Config};
use anyhow::{anyhow, Result};

pub fn select(
    config: &Config,
    notes: &[PathBuf],
    files: &[String],
    diff: Option<&str>,
) -> Result<Option<BTreeSet<String>>> {
    if files.is_empty() && diff.is_none() {
        return Ok(None);
    }
    if !files.is_empty() && diff.is_some() {
        return Err(anyhow!("files and --diff cannot be combined"));
    }
    let root = config.vault_path.canonicalize()?;
    let available: BTreeSet<String> = notes
        .iter()
        .map(|p| relative_path(&config.vault_path, p))
        .collect();
    let mut selected = BTreeSet::new();
    if let Some(reference) = diff {
        let git_root = git(&root, &["rev-parse", "--show-toplevel"])?;
        let git_root =
            PathBuf::from(std::str::from_utf8(&git_root)?.trim_end_matches(['\r', '\n']))
                .canonicalize()?;
        let mut changed = Vec::new();
        if reference.is_empty() {
            changed.extend(git(
                &root,
                &[
                    "diff",
                    "--name-only",
                    "--no-renames",
                    "--no-relative",
                    "-z",
                    "--",
                ],
            )?);
            changed.extend(git(
                &root,
                &[
                    "diff",
                    "--cached",
                    "--name-only",
                    "--no-renames",
                    "--no-relative",
                    "-z",
                    "--",
                ],
            )?);
        } else {
            let oid = git(
                &root,
                &[
                    "rev-parse",
                    "--verify",
                    "--end-of-options",
                    &format!("{reference}^{{commit}}"),
                ],
            )?;
            let oid = std::str::from_utf8(&oid)?.trim();
            changed.extend(git(
                &root,
                &[
                    "diff",
                    "--name-only",
                    "--no-renames",
                    "--no-relative",
                    "-z",
                    oid,
                    "--",
                ],
            )?);
        }
        changed.extend(git(
            &root,
            &[
                "ls-files",
                "--others",
                "--exclude-standard",
                "--full-name",
                "-z",
            ],
        )?);
        for name in changed.split(|c| *c == 0).filter(|s| !s.is_empty()) {
            let name = std::str::from_utf8(name)
                .map_err(|_| anyhow!("Git returned a non-UTF-8 filename"))?;
            let path = git_root.join(name);
            if let Ok(relative) = path.strip_prefix(&root) {
                let relative = relative.to_string_lossy().replace('\\', "/");
                if available.contains(&relative) {
                    selected.insert(relative);
                }
            }
        }
    } else {
        for file in files {
            let path = resolve_path(&config.vault_path, file)
                .canonicalize()
                .map_err(|err| anyhow!("cannot select {file:?}: {err}"))?;
            let relative = path
                .strip_prefix(&root)
                .map_err(|_| anyhow!("file is outside the vault: {file}"))?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            if !available.contains(&relative) {
                return Err(anyhow!("not an included note: {file}"));
            }
            selected.insert(relative);
        }
    }
    Ok(Some(selected))
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(anyhow!(
            "Git selection failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}
