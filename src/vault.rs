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
    pub template_engine: String,
    pub flavor: String,
    pub exclude: Vec<String>,
    pub ignore_links: Vec<String>,
    pub daily_folder: String,
    /// The template a daily note is created from, exactly as the vault wrote
    /// it. `None` means the vault named none: only then is a bare dated note
    /// an acceptable answer, because nobody asked for anything else.
    pub daily_template: Option<String>,
    pub daily_format: String,
    pub tasks_default_file: String,
    pub tasks_inbox: String,
    pub tasks_created_date: bool,
    pub tasks_created_date_format: String,
    pub tasks_statuses: std::collections::BTreeMap<String, StatusOverride>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            vault_path: PathBuf::from("."),
            template_engine: "templater".into(),
            flavor: "markdown".into(),
            exclude: Vec::new(),
            ignore_links: Vec::new(),
            daily_folder: "Daily".into(),
            daily_template: None,
            daily_format: "YYYY-MM-DD".into(),
            tasks_default_file: "daily".into(),
            tasks_inbox: "Inbox/Tasks.md".into(),
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

// ----------------------------------------------------------- The schema --
//
// A note is read leniently: a header that does not parse costs that note its
// frontmatter and nothing else, because one bad file must not take down a
// whole-vault command. `knapper.config.md` gets the opposite treatment. It
// decides what every command does to every note, so a misspelt key there is
// not a key that does nothing -- it is `exclude` silently switching off, or a
// task marker silently reverting, with no output to say so. Everything below
// exists to turn that into an error that names the key.

/// What one setting may hold.
enum Shape {
    /// A string. `Some(values)` closes it to a set, matched case-insensitively.
    Text(Option<&'static [&'static str]>),
    Bool,
    /// A string, or a list of strings.
    TextList,
    /// A block with a fixed set of keys.
    Fields(&'static [(&'static str, Shape)]),
    /// A block whose keys the vault chooses, each holding the same fields.
    Named(&'static [(&'static str, Shape)]),
}

/// The engines `templater::expand` actually implements.
pub const TEMPLATE_ENGINES: &[&str] = &["templater", "core"];

/// The flavors that change how a note is read. knapper supports more
/// ecosystems than this -- Foam, Dendron, Roam, org-mode -- but they need no
/// setting, so accepting their names here would promise a switch that does
/// nothing.
pub const FLAVORS: &[&str] = &["markdown", "logseq"];

const STATUS: &[(&str, Shape)] = &[
    ("char", Shape::Text(None)),
    ("closed", Shape::Bool),
    ("date_format", Shape::Text(None)),
];

const TASKS: &[(&str, Shape)] = &[
    ("default_file", Shape::Text(None)),
    ("inbox", Shape::Text(None)),
    ("created_date", Shape::Bool),
    ("created_date_format", Shape::Text(None)),
    ("statuses", Shape::Named(STATUS)),
];

const DAILY_NOTES: &[(&str, Shape)] = &[
    ("folder", Shape::Text(None)),
    ("template", Shape::Text(None)),
    ("format", Shape::Text(None)),
];

const SETTINGS: &[(&str, Shape)] = &[
    ("vault_path", Shape::Text(None)),
    ("template_engine", Shape::Text(Some(TEMPLATE_ENGINES))),
    ("flavor", Shape::Text(Some(FLAVORS))),
    ("exclude", Shape::TextList),
    ("ignore_links", Shape::TextList),
    ("daily_notes", Shape::Fields(DAILY_NOTES)),
    ("tasks", Shape::Fields(TASKS)),
];

/// How a value reads in an error, in the terms the config file uses.
fn yaml_kind(value: &serde_yaml::Value) -> &'static str {
    match value {
        serde_yaml::Value::Null => "empty",
        serde_yaml::Value::Bool(_) => "true/false",
        serde_yaml::Value::Number(_) => "a number",
        serde_yaml::Value::String(_) => "a string",
        serde_yaml::Value::Sequence(_) => "a list",
        serde_yaml::Value::Mapping(_) => "a block of settings",
        serde_yaml::Value::Tagged(_) => "a tagged value",
    }
}

fn unknown_setting(path: &str, fields: &[(&str, Shape)]) -> anyhow::Error {
    // A vault travels: it is synced, shared, cloned and handed over. Nothing
    // it declares may be executable, so `providers:` is refused by name
    // rather than skipped. Skipping it is the more dangerous answer -- the
    // vault would look configured, and resolve nothing, without saying why.
    if path == "providers" {
        return anyhow!(
            "`providers` is not vault configuration and is never run from a vault. \
             A provider command belongs to a machine, not to a set of notes: write it \
             with `knapper providers set NAME -- COMMAND`, which keeps it in the local \
             provider config outside the vault."
        );
    }
    let known: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    anyhow!(
        "unknown setting `{path}`. Settings here: {}",
        known.join(", ")
    )
}

fn check_block(
    prefix: Option<&str>,
    block: &serde_yaml::Mapping,
    fields: &[(&str, Shape)],
) -> Result<()> {
    for (key, value) in block {
        let Some(key) = key.as_str() else {
            return Err(anyhow!(
                "settings are named by strings, but one key is {}",
                yaml_kind(key)
            ));
        };
        let path = match prefix {
            Some(prefix) => format!("{prefix}.{key}"),
            None => key.to_string(),
        };
        let Some((_, shape)) = fields.iter().find(|(name, _)| *name == key) else {
            return Err(unknown_setting(&path, fields));
        };
        check_value(&path, value, shape)?;
    }
    Ok(())
}

fn check_value(path: &str, value: &serde_yaml::Value, shape: &Shape) -> Result<()> {
    // A key with nothing under it is a key the vault did not set: `exclude:`
    // on its own, or `date_format:` clearing a default. Both are answers, not
    // mistakes, and the loader below already reads them as such.
    if value.is_null() {
        return Ok(());
    }
    match shape {
        Shape::Text(allowed) => {
            let Some(text) = value.as_str() else {
                return Err(anyhow!(
                    "`{path}` must be a string, but it is {}",
                    yaml_kind(value)
                ));
            };
            match allowed {
                Some(allowed) if !allowed.iter().any(|v| v.eq_ignore_ascii_case(text)) => {
                    Err(anyhow!(
                        "`{path}` must be {}, but it is `{text}`",
                        allowed.join(" or ")
                    ))
                }
                _ => Ok(()),
            }
        }
        Shape::Bool => match value.as_bool() {
            Some(_) => Ok(()),
            None => Err(anyhow!(
                "`{path}` must be true or false, but it is {}",
                yaml_kind(value)
            )),
        },
        Shape::TextList => match value {
            serde_yaml::Value::String(_) => Ok(()),
            serde_yaml::Value::Sequence(items) => {
                for (index, item) in items.iter().enumerate() {
                    if !item.is_string() {
                        return Err(anyhow!(
                            "`{path}` entry {} must be a string, but it is {}",
                            index + 1,
                            yaml_kind(item)
                        ));
                    }
                }
                Ok(())
            }
            other => Err(anyhow!(
                "`{path}` must be a string or a list of strings, but it is {}",
                yaml_kind(other)
            )),
        },
        Shape::Fields(fields) => match value.as_mapping() {
            Some(block) => check_block(Some(path), block, fields),
            None => Err(anyhow!(
                "`{path}` must be a block of settings, but it is {}",
                yaml_kind(value)
            )),
        },
        Shape::Named(fields) => {
            let Some(block) = value.as_mapping() else {
                return Err(anyhow!(
                    "`{path}` must be a block of settings, but it is {}",
                    yaml_kind(value)
                ));
            };
            for (name, entry) in block {
                let Some(name) = name.as_str() else {
                    return Err(anyhow!(
                        "`{path}` is keyed by names, but one of its keys is {}",
                        yaml_kind(name)
                    ));
                };
                check_value(&format!("{path}.{name}"), entry, &Shape::Fields(fields))?;
            }
            Ok(())
        }
    }
}

/// Check a whole config against the schema.
pub fn validate(settings: &serde_yaml::Mapping) -> Result<()> {
    check_block(None, settings, SETTINGS)
}

/// Read the config's settings, refusing what `split_frontmatter` forgives.
fn config_settings(raw: &str) -> Result<serde_yaml::Mapping> {
    let Some((header, _)) = crate::note::split_frontmatter_raw(raw) else {
        return Err(anyhow!(
            "the settings live in a `---` block at the top of the file, and there is none here"
        ));
    };
    match serde_yaml::from_str::<serde_yaml::Value>(header) {
        Ok(serde_yaml::Value::Mapping(settings)) => Ok(settings),
        // A config that declares nothing is a config, and every default
        // applies. It is a header that means something else that is refused.
        Ok(serde_yaml::Value::Null) => Ok(serde_yaml::Mapping::new()),
        Ok(other) => Err(anyhow!(
            "the `---` block must be a set of settings, but it is {}",
            yaml_kind(&other)
        )),
        Err(err) => Err(anyhow!("the `---` block is not valid YAML: {err}")),
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
    // Every error from here names the file, so one message is enough to act
    // on however the config was found -- walked up to, or passed with -c.
    let meta = config_settings(&raw)
        .and_then(|settings| validate(&settings).map(|()| settings))
        .map_err(|err| anyhow!("{}: {err}", path.display()))?;

    let get = |key: &str| meta.get(serde_yaml::Value::String(key.into()));
    let get_str = |key: &str, fallback: &str| {
        get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| fallback.to_string())
    };

    let daily = get("daily_notes").and_then(|v| v.as_mapping());
    let daily_raw = |key: &str| {
        daily
            .and_then(|m| m.get(serde_yaml::Value::String(key.into())))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    let daily_get = |key: &str, fallback: &str| daily_raw(key).unwrap_or_else(|| fallback.into());

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
        tasks_default_file: tasks_str("default_file", "daily"),
        tasks_inbox: tasks_str("inbox", "Inbox/Tasks.md"),
        tasks_created_date: tasks_bool("created_date", true),
        tasks_created_date_format: tasks_str("created_date_format", "➕ YYYY-MM-DD"),
        tasks_statuses,
        // Both are validated case-insensitively, so both are lowered here:
        // an accepted `Core` that then behaved as templater would be the
        // silent fallback this validation exists to remove.
        template_engine: get_str("template_engine", "templater").to_ascii_lowercase(),
        flavor: get_str("flavor", "markdown").to_ascii_lowercase(),
        exclude: as_string_list(get("exclude")),
        ignore_links: as_string_list(get("ignore_links")),
        daily_folder: daily_get("folder", "Daily"),
        daily_template: daily_raw("template"),
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

/// Every file in the vault, newest first.
///
/// This is deliberately broader than `all_notes`: the link graph needs to
/// know about excluded notes and leaf attachments without parsing either as
/// source notes. Callers that need the user's note set should use
/// `all_notes`, which applies extension and exclude filtering below.
pub fn all_files(config: &Config) -> Vec<PathBuf> {
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
            // knapper's own config is not one of the user's notes.
            if path.file_name()?.to_str()? == CONFIG_FILENAME {
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

/// Every note in the vault, newest first.
pub fn all_notes(config: &Config) -> Vec<PathBuf> {
    all_files(config)
        .into_iter()
        .filter(|path| {
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                return false;
            };
            if !DEFAULT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
                return false;
            }
            let relative = relative_path(&config.vault_path, path);
            !is_excluded(&relative, &config.exclude)
        })
        .collect()
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
    use std::collections::BTreeSet;
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
    fn all_files_keeps_excluded_notes_and_leaf_files_for_target_discovery() {
        let dir = vault_with(&["Source.md", "logs/Excluded.md", "assets/paper.pdf"]);
        let config = Config {
            vault_path: dir.path().to_path_buf(),
            exclude: excludes(&["logs"]),
            ..Default::default()
        };
        let files: BTreeSet<_> = all_files(&config)
            .into_iter()
            .map(|path| relative_path(&config.vault_path, &path))
            .collect();
        assert!(files.contains("logs/Excluded.md"));
        assert!(files.contains("assets/paper.pdf"));
        assert!(!all_notes(&config)
            .iter()
            .any(|path| { relative_path(&config.vault_path, path) == "logs/Excluded.md" }));
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

    /// `ignore_links` names link targets rather than paths, but a vault
    /// declares it the same way it declares `exclude`.
    #[test]
    fn ignore_links_is_read_as_either_a_scalar_or_a_list() {
        for (yaml, expected) in [
            (
                "ignore_links:\n  - Daily Tasks\n  - \"[[Habits]]\"",
                vec!["Daily Tasks", "[[Habits]]"],
            ),
            ("ignore_links: Daily Tasks", vec!["Daily Tasks"]),
            ("ignore_links:", vec![]),
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
            assert_eq!(config.ignore_links, excludes(&expected), "yaml: {yaml:?}");
        }
    }

    // ------------------------------------------------- Strict validation --

    /// Load a config written verbatim, header and all.
    fn load(raw: &str) -> Result<Config> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILENAME);
        fs::write(&path, raw).unwrap();
        load_config(Some(path.to_str().unwrap()), None)
    }

    /// Load settings written inside a `---` block, which is the usual case.
    fn settings(yaml: &str) -> Result<Config> {
        load(&format!("---\n{yaml}\n---\n"))
    }

    /// Every rejection must name the thing that is wrong. An error that says
    /// only "invalid config" leaves the reader diffing their file by hand.
    fn refused(yaml: &str, needles: &[&str]) {
        let err = match settings(yaml) {
            Ok(_) => panic!("accepted: {yaml:?}"),
            Err(err) => format!("{err}"),
        };
        for needle in needles {
            assert!(err.contains(needle), "{yaml:?} -> {err:?} lacks {needle:?}");
        }
        assert!(
            err.contains(CONFIG_FILENAME),
            "{yaml:?} -> {err:?} does not say which file"
        );
    }

    /// A key knapper does not know is a key that does nothing, and a key that
    /// does nothing is indistinguishable from a setting that stopped working.
    #[test]
    fn an_unknown_setting_is_named_and_refused() {
        refused("templates:\n  folder: Templates", &["`templates`"]);
        refused("vault_path: .\nexcludes:\n  - logs", &["`excludes`"]);
        // The error also says what could have been meant instead.
        refused("excludes:\n  - logs", &["exclude", "ignore_links"]);
    }

    /// A typo one level down is the one that hides best: the block it is in
    /// still parses, still applies, and quietly drops the line.
    #[test]
    fn an_unknown_nested_setting_is_named_by_its_path() {
        refused(
            "daily_notes:\n  formt: YYYY-MM-DD",
            &["`daily_notes.formt`"],
        );
        refused("tasks:\n  default_fil: daily", &["`tasks.default_fil`"]);
        refused(
            "tasks:\n  statuses:\n    done:\n      dateformat: x",
            &["`tasks.statuses.done.dateformat`"],
        );
        // The keys that were removed rather than never added are refused the
        // same way, with no compatibility path behind them.
        refused("tasks:\n  done_date: false", &["`tasks.done_date`"]);
        refused(
            "tasks:\n  done_date_format: \"✅ YYYY-MM-DD\"",
            &["`tasks.done_date_format`"],
        );
    }

    /// The old loader read a wrong type as an absent one, so `exclude: true`
    /// excluded nothing and said nothing.
    #[test]
    fn a_setting_of_the_wrong_type_is_refused_rather_than_ignored() {
        refused("exclude: true", &["`exclude`", "list of strings"]);
        refused(
            "exclude:\n  - logs\n  - 7",
            &["`exclude` entry 2", "number"],
        );
        refused(
            "tasks:\n  created_date: yes please",
            &["`tasks.created_date`", "true or false"],
        );
        refused(
            "daily_notes: Daily",
            &["`daily_notes`", "block of settings"],
        );
        refused("vault_path:\n  - .", &["`vault_path`", "must be a string"]);
        refused(
            "tasks:\n  statuses:\n    done:\n      closed: sometimes",
            &["`tasks.statuses.done.closed`"],
        );
    }

    /// Naming an engine or flavor knapper does not implement used to select
    /// the default one, so the config claimed a behaviour it never got.
    #[test]
    fn an_unsupported_engine_or_flavor_is_refused() {
        refused(
            "template_engine: jinja",
            &["`template_engine`", "templater or core"],
        );
        refused("flavor: obsidian", &["`flavor`", "markdown or logseq"]);
        // The supported ones pass, in any case, and reach the config lowered
        // so nothing downstream compares against the wrong spelling.
        assert_eq!(
            settings("template_engine: Core").unwrap().template_engine,
            "core"
        );
        assert_eq!(settings("flavor: LogSeq").unwrap().flavor, "logseq");
    }

    /// A note survives a header that does not parse. The file that decides
    /// which notes exist does not get the same forgiveness.
    #[test]
    fn a_malformed_config_is_refused_where_a_note_would_be_forgiven() {
        for raw in [
            "---\nkey: value: nested\n---\n",
            "---\n\tbad indent\n---\n",
            "---\ntitle: \u{2}broken\n---\n",
        ] {
            let err = format!("{}", load(raw).unwrap_err());
            assert!(err.contains("not valid YAML"), "{raw:?} -> {err:?}");
            // The same header costs an ordinary note only its frontmatter.
            let (frontmatter, _) = crate::note::split_frontmatter(raw);
            assert!(frontmatter.is_empty(), "{raw:?}");
        }

        let err = format!("{}", load("# Just prose\n").unwrap_err());
        assert!(err.contains("`---` block"), "{err:?}");

        let err = format!("{}", load("---\n- a list\n---\n").unwrap_err());
        assert!(err.contains("set of settings"), "{err:?}");
    }

    /// A vault is synced, shared and cloned, so nothing it declares may be
    /// executable. Refusing the block by name beats skipping it: a skipped
    /// block leaves the vault looking configured.
    #[test]
    fn a_providers_block_is_refused_with_somewhere_else_to_put_it() {
        refused(
            "vault_path: .\nproviders:\n  personal:\n    command: [op, read, x]",
            &[
                "`providers` is not vault configuration",
                "never run from a vault",
                "knapper providers set",
            ],
        );
    }

    /// A config that declares nothing is still a config: strictness is about
    /// what a vault says, not about making it say something.
    #[test]
    fn an_empty_config_is_accepted_and_every_default_applies() {
        for raw in ["---\n\n---\n", "---\n# only a comment\n---\n"] {
            let config = load(raw).unwrap_or_else(|e| panic!("{raw:?}: {e}"));
            assert_eq!(config.template_engine, "templater");
            assert_eq!(config.flavor, "markdown");
            assert_eq!(config.daily_template, None);
            assert!(config.exclude.is_empty());
        }
    }

    /// The whole vocabulary, in one config, so the schema cannot drift out
    /// from under a setting that is still documented.
    #[test]
    fn every_documented_setting_is_accepted() {
        let config = settings(
            "vault_path: .\n\
             template_engine: core\n\
             flavor: markdown\n\
             exclude:\n  - Templates/\n\
             ignore_links:\n  - Daily Tasks\n\
             daily_notes:\n  folder: Diary\n  template: assets/daily.md\n  format: YYYY/MM-DD\n\
             tasks:\n\
             \x20 default_file: inbox\n\
             \x20 inbox: Inbox/Tasks.md\n\
             \x20 created_date: false\n\
             \x20 created_date_format: \"➕ YYYY-MM-DD\"\n\
             \x20 statuses:\n\
             \x20   done:\n      date_format: null\n\
             \x20   forward:\n      char: \">\"\n      closed: true\n",
        )
        .unwrap();

        assert_eq!(config.template_engine, "core");
        assert_eq!(config.daily_folder, "Diary");
        assert_eq!(config.daily_template.as_deref(), Some("assets/daily.md"));
        assert_eq!(config.daily_format, "YYYY/MM-DD");
        assert_eq!(config.tasks_default_file, "inbox");
        assert!(!config.tasks_created_date);
        assert!(config.tasks_statuses["done"].date_format_set);
        assert_eq!(config.tasks_statuses["forward"].char, Some('>'));
    }

    /// `knapper init` must write a config `knapper init` can then read. The
    /// generated file is prose as well as settings, so nothing else checks
    /// that its frontmatter still answers to the schema.
    #[test]
    fn the_generated_config_validates_against_the_schema() {
        let config = load(crate::notes_cmd::DEFAULT_CONFIG)
            .unwrap_or_else(|err| panic!("knapper init writes an invalid config: {err}"));
        // It configures a template folder, so it excludes that folder: no
        // folder name is special to knapper any more.
        assert!(
            config
                .exclude
                .iter()
                .any(|e| e.trim_end_matches('/') == "Templates"),
            "the generated config no longer excludes its own template folder: {:?}",
            config.exclude
        );
        assert_eq!(
            config.daily_template.as_deref(),
            Some("Templates/daily.md"),
            "the generated template path and the generated exclude have drifted apart"
        );
    }
}
