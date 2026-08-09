//! The link graph, and the resolver that turns a link target into a path.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Component, Path};

use rayon::prelude::*;

use crate::note::parse_note;
use crate::org;
use crate::parser::strip_anchor;
use crate::vault::{all_files, all_notes, is_org, relative_path, Config, DEFAULT_EXTENSIONS};

/// Resolves link targets to vault-relative paths.
///
/// Builds its lookup tables once, so resolving is a map hit rather than a scan
/// of every file. Resolving per link against the whole file list is
/// O(links x files) and becomes unusable well before a vault gets large.
pub struct LinkResolver {
    /// All vault files, including excluded notes and non-note leaf files.
    by_path: HashMap<String, String>,
    /// Basename lookup remains note-only. A generic attachment basename is
    /// too ambiguous to resolve safely; attachments resolve by path.
    by_stem: HashMap<String, String>,
    by_alias: HashMap<String, String>,
    // org only: :ID: properties, and heading text for [[*Heading]] links,
    // which org resolves across every file.
    by_id: HashMap<String, String>,
    by_heading: HashMap<String, String>,
}

impl LinkResolver {
    pub fn new(
        files: BTreeSet<String>,
        target_files: BTreeSet<String>,
        aliases: BTreeMap<String, String>,
        ids: BTreeMap<String, String>,
        headings: BTreeMap<String, String>,
    ) -> Self {
        let mut by_path = HashMap::new();
        let mut by_stem = HashMap::new();
        for file in &target_files {
            by_path.entry(file.to_lowercase()).or_insert(file.clone());
        }
        for file in &files {
            let stem = Path::new(file)
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            by_stem.entry(stem).or_insert(file.clone());
        }

        Self {
            by_path,
            by_stem,
            by_alias: aliases
                .into_iter()
                .map(|(a, p)| (a.to_lowercase(), p))
                .collect(),
            by_id: ids.into_iter().collect(),
            by_heading: headings.into_iter().collect(),
        }
    }

    pub fn resolve(&self, source: &str, target: &str) -> Option<String> {
        // org id: and *Heading links carry their sigil through the parser.
        if let Some(id) = target.strip_prefix("id:") {
            return self.by_id.get(id.trim()).cloned();
        }
        if let Some(heading) = target.strip_prefix('*') {
            return self.by_heading.get(&heading.trim().to_lowercase()).cloned();
        }

        if let Some(hit) = self.resolve_path(target) {
            return Some(hit);
        }

        // Markdown-style paths in research indexes are relative to the note
        // that contains them (`survey/foo.md`, `../papers/foo/summary.md`).
        // Try the safe relative path after vault-root resolution, preserving
        // the historical root-path behaviour when both are possible.
        if let Some(relative) = relative_target(source, target) {
            if let Some(hit) = self.resolve_path(&relative) {
                return Some(hit);
            }
        }

        let lower = target.to_lowercase();
        let hit = self
            .by_path
            .get(&format!("{lower}.md"))
            .or_else(|| self.by_path.get(&format!("{lower}.org")))
            .or_else(|| self.by_stem.get(&lower))
            .or_else(|| self.by_alias.get(&lower));
        if let Some(hit) = hit {
            return Some(hit.clone());
        }

        // A path that escaped the vault must never fall through to basename
        // matching: `../outside.md` cannot accidentally resolve to an
        // unrelated in-vault note called `outside`.
        if target.split('/').any(|part| part == "..") && relative_target(source, target).is_none() {
            return None;
        }

        // Relative markdown links such as ../other/Note resolve by basename,
        // the same way a bare wikilink does.
        let basename = Path::new(target)
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())?;
        if basename != lower {
            return self
                .by_stem
                .get(&basename)
                .or_else(|| self.by_alias.get(&basename))
                .cloned();
        }
        None
    }

    fn resolve_path(&self, target: &str) -> Option<String> {
        for suffix in ["md", "org", "markdown", "mdx"] {
            let candidate = format!("{target}.{suffix}");
            if let Some(hit) = self.by_path.get(&candidate.to_lowercase()) {
                return Some(hit.clone());
            }
        }
        self.by_path.get(&target.to_lowercase()).cloned()
    }
}

/// Resolve a target against the containing note's directory without allowing
/// `..` to escape the vault. The result is vault-relative and slash-normalised.
fn relative_target(source: &str, target: &str) -> Option<String> {
    if target.is_empty() || Path::new(target).is_absolute() {
        return None;
    }

    let parent = Path::new(source).parent().unwrap_or_else(|| Path::new(""));
    let mut components: Vec<String> = parent
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            Component::CurDir => None,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => None,
        })
        .collect();

    for component in Path::new(target).components() {
        match component {
            Component::Normal(value) => components.push(value.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                components.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    (!components.is_empty()).then(|| components.join("/"))
}

/// Link targets a vault declares as intentionally unresolved, from the
/// `ignore_links` config key.
///
/// A vault that outlived the tool it came from carries links that will never
/// resolve and are not mistakes: a tag page from an import, a section name a
/// generator emitted. Reporting them once per run buries the broken links that
/// are real, so the vault says so once, in config.
///
/// Matching is on the whole target, never a substring, so an entry can only
/// silence the link it names.
#[derive(Debug, Default)]
pub struct IgnoredLinks {
    targets: HashSet<String>,
}

impl IgnoredLinks {
    pub fn new(entries: &[String]) -> Self {
        Self {
            targets: entries
                .iter()
                .map(|entry| normalize_link_target(entry))
                .filter(|entry| !entry.is_empty())
                .collect(),
        }
    }

    pub fn contains(&self, target: &str) -> bool {
        !self.targets.is_empty() && self.targets.contains(&normalize_link_target(target))
    }
}

/// The comparison form of a link target, applied to both sides so a config
/// entry may be written the way the link is written in a note.
///
/// Drops a `[[...]]` wrapper and a `|display` part, so an entry copied out of
/// a note matches; drops an anchor, a `./` prefix and a note extension, which
/// is what the parser already does to the targets the graph sees; and folds
/// case, because resolution is case-insensitive and an ignore that were not
/// would depend on how each link happened to be capitalised.
fn normalize_link_target(target: &str) -> String {
    let mut target = target.trim();
    if let Some(inner) = target.strip_prefix("[[").and_then(|t| t.strip_suffix("]]")) {
        target = inner;
    }
    if let Some((before, _)) = target.split_once('|') {
        target = before;
    }

    let mut target = strip_anchor(target);
    while let Some(rest) = target.strip_prefix("./") {
        target = rest.to_string();
    }
    if let Some((stem, ext)) = target.rsplit_once('.') {
        if DEFAULT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) && !stem.is_empty() {
            target = stem.to_string();
        }
    }
    target.trim().to_lowercase()
}

#[derive(Debug, Default)]
pub struct LinkGraph {
    pub outgoing: BTreeMap<String, BTreeSet<String>>,
    pub incoming: BTreeMap<String, BTreeSet<String>>,
    pub broken: BTreeMap<String, Vec<String>>,
    pub files: BTreeSet<String>,
}

struct Parsed {
    relative: String,
    links: Vec<String>,
    aliases: Vec<String>,
    ids: Vec<String>,
    headings: Vec<String>,
}

/// Read every note once, and build the resolver from what they declare.
///
/// Aliases must be known before anything is resolved, since `[[an alias]]`
/// points at the note declaring it, so this pass has to finish before the
/// first target is looked up.
fn scan_vault(config: &Config) -> (Vec<Parsed>, BTreeSet<String>, LinkResolver) {
    let paths = all_notes(config);
    let target_files: BTreeSet<String> = all_files(config)
        .iter()
        .map(|path| relative_path(&config.vault_path, path))
        .collect();

    let parsed: Vec<Parsed> = paths
        .par_iter()
        .filter_map(|path| {
            let content = std::fs::read_to_string(path).ok()?;
            let relative = relative_path(&config.vault_path, path);

            if is_org(path) {
                let doc = org::parse_org(&content);
                return Some(Parsed {
                    relative,
                    links: doc.links,
                    aliases: doc.aliases,
                    ids: doc.ids,
                    headings: doc
                        .headings
                        .into_iter()
                        .filter(|h| !h.text.is_empty())
                        .map(|h| h.text.to_lowercase())
                        .collect(),
                });
            }

            let note = parse_note(path, &content);
            Some(Parsed {
                relative,
                links: note.links,
                aliases: note.aliases,
                ids: Vec::new(),
                headings: Vec::new(),
            })
        })
        .collect();

    let files: BTreeSet<String> = parsed.iter().map(|p| p.relative.clone()).collect();

    let mut aliases = BTreeMap::new();
    let mut ids = BTreeMap::new();
    let mut headings = BTreeMap::new();
    for entry in &parsed {
        for alias in &entry.aliases {
            aliases
                .entry(alias.clone())
                .or_insert(entry.relative.clone());
        }
        for id in &entry.ids {
            ids.entry(id.clone()).or_insert(entry.relative.clone());
        }
        for heading in &entry.headings {
            headings
                .entry(heading.clone())
                .or_insert(entry.relative.clone());
        }
    }

    let resolver = LinkResolver::new(files.clone(), target_files, aliases, ids, headings);
    (parsed, files, resolver)
}

/// The resolver on its own, for a caller that has to ask whether one target
/// names a real file without needing the whole graph around it.
pub fn build_resolver(config: &Config) -> LinkResolver {
    scan_vault(config).2
}

/// Build the link graph.
pub fn build_link_graph(config: &Config) -> LinkGraph {
    let (parsed, files, resolver) = scan_vault(config);

    let mut graph = LinkGraph {
        files,
        ..Default::default()
    };

    let ignored = IgnoredLinks::new(&config.ignore_links);

    for entry in &parsed {
        for target in &entry.links {
            match resolver.resolve(&entry.relative, target) {
                Some(resolved) => {
                    graph
                        .outgoing
                        .entry(entry.relative.clone())
                        .or_default()
                        .insert(resolved.clone());
                    graph
                        .incoming
                        .entry(resolved)
                        .or_default()
                        .insert(entry.relative.clone());
                }
                // An unresolved link the config named is not a finding. It
                // still resolves to nothing, so it becomes no edge either.
                _ if ignored.contains(target) => {}
                _ => graph
                    .broken
                    .entry(entry.relative.clone())
                    .or_default()
                    .push(target.clone()),
            }
        }
    }

    graph
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ignoring(entries: &[&str]) -> IgnoredLinks {
        IgnoredLinks::new(&entries.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    /// An ignore silences one target, in every form that target can be
    /// written. The forms below are what reaches the graph after parsing:
    /// an alias and an anchor are already gone, a markdown link has lost its
    /// extension, and resolution folds case.
    #[test]
    fn an_ignore_covers_every_way_of_writing_the_same_target() {
        let ignored = ignoring(&["Daily Tasks"]);
        for target in [
            "Daily Tasks",
            "daily tasks",
            "DAILY TASKS",
            "Daily Tasks.md",
            "./Daily Tasks",
            "Daily Tasks#2026",
        ] {
            assert!(ignored.contains(target), "should ignore {target:?}");
        }
    }

    /// The entry itself may be written the way the link is written in a note,
    /// since that is what a reader will copy.
    #[test]
    fn an_entry_may_be_written_as_a_link() {
        for entry in ["[[Habits]]", "[[Habits|習慣]]", "Habits.md", "  Habits  "] {
            assert!(ignoring(&[entry]).contains("Habits"), "entry {entry:?}");
        }
    }

    /// The failure that matters is an ignore that quietly hides a real broken
    /// link, so matching is on the whole target and nothing else.
    #[test]
    fn an_ignore_does_not_reach_past_the_target_it_names() {
        let ignored = ignoring(&["Daily Tasks"]);
        for target in [
            "Daily Tasks Archive",
            "Old Daily Tasks",
            "Daily",
            "Tasks",
            // Path-qualified links are their own target: knapper resolves
            // them by basename, but an ignore is not a resolution.
            "Archive/Daily Tasks",
            "Daily Tasks/Index",
        ] {
            assert!(!ignored.contains(target), "should not ignore {target:?}");
        }
    }

    /// A path-qualified ignore is available by writing the path.
    #[test]
    fn a_path_qualified_entry_matches_that_path() {
        let ignored = ignoring(&["Archive/Old Index"]);
        assert!(ignored.contains("Archive/Old Index"));
        assert!(ignored.contains("archive/old index.md"));
        assert!(!ignored.contains("Old Index"));
    }

    #[test]
    fn without_entries_nothing_is_ignored() {
        let ignored = ignoring(&[]);
        assert!(!ignored.contains("Daily Tasks"));
        assert!(!ignored.contains(""));
    }

    /// A dot in a note name is part of the name unless it spells a note
    /// extension, the same rule the markdown-link parser follows.
    #[test]
    fn only_a_note_extension_is_dropped_from_a_target() {
        assert!(ignoring(&["proj.knapper.design"]).contains("proj.knapper.design"));
        assert!(ignoring(&["Notes.org"]).contains("Notes"));
        assert!(!ignoring(&["Report.pdf"]).contains("Report"));
    }

    fn resolver(files: &[&str], targets: &[&str]) -> LinkResolver {
        LinkResolver::new(
            files.iter().map(|file| (*file).to_string()).collect(),
            targets.iter().map(|file| (*file).to_string()).collect(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
    }

    #[test]
    fn path_targets_try_vault_root_then_referring_note_directory() {
        let resolver = resolver(
            &["Source.md", "research/index.md", "research/survey/foo.md"],
            &["Source.md", "research/index.md", "research/survey/foo.md"],
        );

        assert_eq!(
            resolver.resolve("Source.md", "Source.md"),
            Some("Source.md".into())
        );
        assert_eq!(
            resolver.resolve("research/index.md", "survey/foo.md"),
            Some("research/survey/foo.md".into())
        );
    }

    #[test]
    fn relative_parent_targets_are_safe_and_vault_relative() {
        let relative = resolver(
            &["research/survey/index.md", "research/papers/foo/summary.md"],
            &["research/survey/index.md", "research/papers/foo/summary.md"],
        );
        assert_eq!(
            relative.resolve("research/survey/index.md", "../papers/foo/summary.md"),
            Some("research/papers/foo/summary.md".into())
        );

        let outside = resolver(&["outside.md"], &["outside.md"]);
        assert_eq!(outside.resolve("index.md", "../outside.md"), None);
        assert_eq!(outside.resolve("index.md", "/outside.md"), None);
    }

    #[test]
    fn excluded_notes_and_leaf_files_are_resolvable_without_note_stems() {
        let resolver = resolver(
            &["Source.md"],
            &[
                "Source.md",
                "logs/Excluded.md",
                "assets/paper.pdf",
                "assets/paper.txt",
                "assets/financials.json",
            ],
        );
        assert_eq!(
            resolver.resolve("Source.md", "logs/Excluded"),
            Some("logs/Excluded.md".into())
        );
        assert_eq!(
            resolver.resolve("Source.md", "assets/paper.pdf"),
            Some("assets/paper.pdf".into())
        );
        assert_eq!(
            resolver.resolve("Source.md", "assets/paper.txt"),
            Some("assets/paper.txt".into())
        );
        assert_eq!(
            resolver.resolve("Source.md", "assets/financials.json"),
            Some("assets/financials.json".into())
        );
        assert_eq!(resolver.resolve("Source.md", "paper"), None);
    }
}
