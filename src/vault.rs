//! Config loading and vault scanning.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use walkdir::WalkDir;

pub const CONFIG_FILENAME: &str = "knapper.config.md";
pub const DEFAULT_EXTENSIONS: &[&str] = &["md", "markdown", "mdx", "org"];

/// A status override or addition from config. `date_format_set` records that
/// the key was present, so an explicit null can clear the default.
#[derive(Debug, Clone, Default)]
pub struct StatusOverride {
    pub char: Option<char>,
    pub closed: Option<bool>,
    pub date_format: Option<String>,
    pub date_format_set: bool,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub vault_path: PathBuf,
    pub config_path: PathBuf,
    pub template_engine: String,
    pub flavor: String,
    pub exclude: Vec<String>,
    pub daily_folder: String,
    pub daily_template: String,
    pub daily_format: String,
    pub tasks_default_file: String,
    pub tasks_inbox: String,
    pub tasks_done_date: bool,
    pub tasks_done_date_format: String,
    pub tasks_created_date: bool,
    pub tasks_created_date_format: String,
    pub tasks_statuses: std::collections::BTreeMap<String, StatusOverride>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            vault_path: PathBuf::from("."),
            config_path: PathBuf::new(),
            template_engine: "templater".into(),
            flavor: "markdown".into(),
            exclude: Vec::new(),
            daily_folder: "Daily".into(),
            daily_template: "Templates/daily.md".into(),
            daily_format: "YYYY-MM-DD".into(),
            tasks_default_file: "daily".into(),
            tasks_inbox: "Inbox/Tasks.md".into(),
            tasks_done_date: true,
            tasks_done_date_format: "✅ YYYY-MM-DD".into(),
            tasks_created_date: true,
            tasks_created_date_format: "➕ YYYY-MM-DD".into(),
            tasks_statuses: Default::default(),
        }
    }
}

fn as_string_list(value: Option<&serde_yaml::Value>) -> Vec<String> {
    match value {
        Some(serde_yaml::Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
        Some(serde_yaml::Value::Sequence(items)) => items
            .iter()
            .filter_map(|i| i.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Walk up from `start` looking for the config, then fall back to the home
/// directory, as the Python implementation does.
pub fn find_config(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join(CONFIG_FILENAME);
        if candidate.exists() {
            return Some(candidate);
        }
        if !current.pop() {
            break;
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let candidate = home.join(CONFIG_FILENAME);
    candidate.exists().then_some(candidate)
}

pub fn load_config(explicit: Option<&str>, vault_override: Option<&str>) -> Result<Config> {
    let path = match explicit {
        Some(p) => PathBuf::from(p),
        None => find_config(&std::env::current_dir()?)
            .ok_or_else(|| anyhow!("Config file not found. Run 'knapper init' to create one."))?,
    };
    if !path.exists() {
        return Err(anyhow!(
            "Config file not found. Run 'knapper init' to create one."
        ));
    }

    let raw = std::fs::read_to_string(&path)?;
    let (meta, _) = crate::note::split_frontmatter(&raw);

    let get = |key: &str| meta.get(serde_yaml::Value::String(key.into()));
    let get_str = |key: &str, fallback: &str| {
        get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| fallback.to_string())
    };

    let daily = get("daily_notes").and_then(|v| v.as_mapping());
    let daily_get = |key: &str, fallback: &str| {
        daily
            .and_then(|m| m.get(serde_yaml::Value::String(key.into())))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| fallback.to_string())
    };

    let tasks = get("tasks").and_then(|v| v.as_mapping());
    let tasks_get = |key: &str| tasks.and_then(|m| m.get(serde_yaml::Value::String(key.into())));
    let tasks_str = |key: &str, fallback: &str| {
        tasks_get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| fallback.to_string())
    };
    let tasks_bool =
        |key: &str, fallback: bool| tasks_get(key).and_then(|v| v.as_bool()).unwrap_or(fallback);

    let mut tasks_statuses = std::collections::BTreeMap::new();
    if let Some(map) = tasks_get("statuses").and_then(|v| v.as_mapping()) {
        for (name, attrs) in map {
            let (Some(name), Some(attrs)) = (name.as_str(), attrs.as_mapping()) else {
                continue;
            };
            let field = |k: &str| attrs.get(serde_yaml::Value::String(k.into()));
            tasks_statuses.insert(
                name.to_string(),
                StatusOverride {
                    char: field("char")
                        .and_then(|v| v.as_str())
                        .and_then(|s| (s.chars().count() == 1).then(|| s.chars().next().unwrap())),
                    closed: field("closed").and_then(|v| v.as_bool()),
                    date_format: field("date_format")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    date_format_set: field("date_format").is_some(),
                },
            );
        }
    }

    let mut config = Config {
        config_path: path.clone(),
        tasks_default_file: tasks_str("default_file", "daily"),
        tasks_inbox: tasks_str("inbox", "Inbox/Tasks.md"),
        tasks_done_date: tasks_bool("done_date", true),
        tasks_done_date_format: tasks_str("done_date_format", "✅ YYYY-MM-DD"),
        tasks_created_date: tasks_bool("created_date", true),
        tasks_created_date_format: tasks_str("created_date_format", "➕ YYYY-MM-DD"),
        tasks_statuses,
        template_engine: get_str("template_engine", "templater"),
        flavor: get_str("flavor", "markdown").to_ascii_lowercase(),
        exclude: as_string_list(get("exclude")),
        daily_folder: daily_get("folder", "Daily"),
        daily_template: daily_get("template", "Templates/daily.md"),
        daily_format: daily_get("format", "YYYY-MM-DD"),
        ..Default::default()
    };

    let declared = get("vault_path").and_then(|v| v.as_str()).unwrap_or("");
    let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    config.vault_path = match vault_override {
        Some(v) => PathBuf::from(v),
        None if declared.is_empty() || declared == "." => parent,
        None => PathBuf::from(shellexpand(declared)),
    };

    Ok(config)
}

fn shellexpand(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var("HOME")
            .map(|h| format!("{h}/{rest}"))
            .unwrap_or_else(|_| path.to_string()),
        None => path.to_string(),
    }
}

/// True if a vault-relative path is covered by an exclude entry.
pub fn is_excluded(relative: &str, excludes: &[String]) -> bool {
    excludes.iter().any(|entry| {
        let entry = entry.trim().trim_end_matches('/');
        !entry.is_empty() && (relative == entry || relative.starts_with(&format!("{entry}/")))
    })
}

pub fn is_org(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("org")
}

pub fn relative_path(vault: &Path, file: &Path) -> String {
    file.strip_prefix(vault)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Every note in the vault, newest first.
pub fn all_notes(config: &Config) -> Vec<PathBuf> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = WalkDir::new(&config.vault_path)
        .into_iter()
        .filter_entry(|e| {
            // Hidden directories are skipped wholesale, which also keeps the
            // walker out of .git and .obsidian.
            !e.file_name()
                .to_str()
                .map(|n| n.starts_with('.') && e.depth() > 0)
                .unwrap_or(false)
        })
        .filter_map(|e| e.ok())
        // A symlinked *file* is still a note, so stat through the link. A
        // symlinked *directory* is not descended into: it can point outside
        // the vault or back into it, and following it both duplicates notes
        // and risks a cycle.
        .filter(|e| e.path().is_file())
        .filter_map(|entry| {
            let path = entry.path();
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            if !DEFAULT_EXTENSIONS.contains(&ext.as_str()) {
                return None;
            }
            // knapper's own config is not one of the user's notes.
            if path.file_name()?.to_str()? == CONFIG_FILENAME {
                return None;
            }
            let relative = relative_path(&config.vault_path, path);
            if is_excluded(&relative, &config.exclude) {
                return None;
            }
            let mtime = std::fs::metadata(path)
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::UNIX_EPOCH);
            Some((mtime, path.to_path_buf()))
        })
        .collect();

    files.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
    files.into_iter().map(|(_, p)| p).collect()
}

/// Resolve a user-supplied file argument against the vault.
pub fn resolve_path(vault: &Path, file: &str) -> PathBuf {
    let path = Path::new(file);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let direct = vault.join(file);
    if direct.exists() {
        return direct;
    }
    for ext in ["md", "org"] {
        let with_ext = vault.join(format!("{file}.{ext}"));
        if with_ext.exists() {
            return with_ext;
        }
    }
    direct
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn excludes(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// A vault can contain whole subtrees that are notes only by file
    /// extension: imported archives, generated logs. Excluding them has to
    /// happen before anything reads the files.
    #[test]
    fn exclude_matches_path_prefixes_not_substrings() {
        for (path, patterns, expected) in [
            ("logs/a.md", vec!["logs"], true),
            ("logs/a.md", vec!["logs/"], true),
            ("logs/deep/a.md", vec!["logs"], true),
            ("logs.md", vec!["logs"], false),
            ("logsX/a.md", vec!["logs"], false),
            ("notes/a.md", vec!["logs"], false),
            ("a/b/c.md", vec!["a/b"], true),
            ("exact.md", vec!["exact.md"], true),
            ("notes/a.md", vec![], false),
            ("notes/a.md", vec!["", "  "], false),
            // Case-sensitive, as written.
            ("Logs/a.md", vec!["logs"], false),
        ] {
            assert_eq!(
                is_excluded(path, &excludes(&patterns)),
                expected,
                "path {path} against {patterns:?}"
            );
        }
    }

    fn vault_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in files {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        dir
    }

    fn names(config: &Config) -> std::collections::BTreeSet<String> {
        all_notes(config)
            .iter()
            .map(|p| relative_path(&config.vault_path, p))
            .collect()
    }

    #[test]
    fn only_note_extensions_are_scanned() {
        let dir = vault_with(&["a.md", "b.markdown", "c.mdx", "d.org", "e.txt", "f.png"]);
        let config = Config {
            vault_path: dir.path().to_path_buf(),
            ..Default::default()
        };
        assert_eq!(
            names(&config),
            ["a.md", "b.markdown", "c.mdx", "d.org"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        );
    }

    #[test]
    fn an_excluded_subtree_never_reaches_the_scanner() {
        let dir = vault_with(&["logs/a.md", "notes/b.md", "top.md"]);
        let config = Config {
            vault_path: dir.path().to_path_buf(),
            exclude: excludes(&["logs"]),
            ..Default::default()
        };
        assert_eq!(
            names(&config),
            ["notes/b.md", "top.md"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        );
    }

    #[test]
    fn without_excludes_everything_is_scanned() {
        let dir = vault_with(&["logs/a.md", "top.md"]);
        let config = Config {
            vault_path: dir.path().to_path_buf(),
            ..Default::default()
        };
        assert_eq!(all_notes(&config).len(), 2);
    }

    #[test]
    fn the_config_file_is_not_itself_a_note() {
        let dir = vault_with(&[CONFIG_FILENAME, "real.md"]);
        let config = Config {
            vault_path: dir.path().to_path_buf(),
            ..Default::default()
        };
        assert_eq!(
            names(&config),
            ["real.md".to_string()].into_iter().collect()
        );
    }

    #[test]
    fn exclude_is_read_as_either_a_scalar_or_a_list() {
        for (yaml, expected) in [
            (
                "exclude:\n  - logs/\n  - Archives/",
                vec!["logs/", "Archives/"],
            ),
            ("exclude: logs/", vec!["logs/"]),
            ("exclude:", vec![]),
            ("", vec![]),
        ] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(
                dir.path().join(CONFIG_FILENAME),
                format!("---\nvault_path: .\n{yaml}\n---\n"),
            )
            .unwrap();

            let config = load_config(
                Some(dir.path().join(CONFIG_FILENAME).to_str().unwrap()),
                None,
            )
            .unwrap();
            assert_eq!(config.exclude, excludes(&expected), "yaml: {yaml:?}");
        }
    }
}
