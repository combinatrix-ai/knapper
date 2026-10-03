//! Read-only, explicitly scoped adapters for Obsidian core-plugin data.
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

fn scoped_file(vault: &Path, path: &Path) -> Result<Option<PathBuf>> {
    match path.canonicalize() {
        Ok(resolved) if resolved.starts_with(vault) => Ok(Some(resolved)),
        Ok(_) => Err(anyhow!(
            "Obsidian settings must remain inside the vault: {}",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("Resolving {}", path.display())),
    }
}

fn json_file(path: &Path) -> Result<Value> {
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("Reading {}", path.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("Invalid Obsidian JSON: {}", path.display()))
}

/// Dataview's StarredCache includes only file entries, recursively in groups.
/// File subpaths still star the file; folder/search/graph/URL items are ignored.
pub fn bookmarked_files(vault: &Path, config_dir: &str) -> Result<Vec<String>> {
    let directory = Path::new(config_dir);
    if directory.as_os_str().is_empty()
        || directory.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(anyhow!(
            "--obsidian-config-dir must be a vault-relative directory without '..'"
        ));
    }
    let root = vault.canonicalize()?;
    let config = root.join(directory);
    let Some(bookmarks) = scoped_file(&root, &config.join("bookmarks.json"))? else {
        return Ok(Vec::new());
    };
    if let Some(core) = scoped_file(&root, &config.join("core-plugins.json"))? {
        let settings = json_file(&core)?;
        let enabled = match &settings {
            Value::Array(values) if values.iter().all(Value::is_string) => {
                values.iter().any(|v| v == "bookmarks")
            }
            Value::Object(values) if values.values().all(Value::is_boolean) => values
                .get("bookmarks")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            _ => {
                return Err(anyhow!(
                    "Invalid core-plugins.json: expected plugin IDs or boolean plugin states"
                ))
            }
        };
        if !enabled {
            return Ok(Vec::new());
        }
    }
    let data = json_file(&bookmarks)?;
    let items = data
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Invalid bookmarks.json: 'items' must be an array"))?;
    let mut files = BTreeSet::new();
    // Avoid recursive Rust stack growth for deeply nested bookmark groups.
    let mut pending: Vec<&Value> = items.iter().collect();
    while let Some(item) = pending.pop() {
        let kind = item
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Invalid bookmarks.json: item 'type' must be a string"))?;
        match kind {
            "group" => {
                if let Some(children) = item.get("items") {
                    let children = children.as_array().ok_or_else(|| {
                        anyhow!("Invalid bookmarks.json: group 'items' must be an array")
                    })?;
                    pending.extend(children);
                }
            }
            "file" => {
                let path = item
                    .get("path")
                    .and_then(Value::as_str)
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| {
                        anyhow!("Invalid bookmarks.json: file 'path' must be a nonempty string")
                    })?;
                files.insert(path.to_owned());
            }
            _ => {}
        }
    }
    Ok(files.into_iter().collect())
}
