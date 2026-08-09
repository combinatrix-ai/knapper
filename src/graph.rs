//! The link graph, and the resolver that turns a link target into a path.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use rayon::prelude::*;

use crate::note::parse_note;
use crate::org;
use crate::parser::strip_anchor;
use crate::vault::{all_notes, is_org, relative_path, Config, DEFAULT_EXTENSIONS};

/// Resolves link targets to vault-relative paths.
///
/// Builds its lookup tables once, so resolving is a map hit rather than a scan
/// of every file. Resolving per link against the whole file list is
/// O(links x files) and becomes unusable well before a vault gets large.
pub struct LinkResolver {
    exact: BTreeSet<String>,
    by_path: HashMap<String, String>,
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
        aliases: BTreeMap<String, String>,
        ids: BTreeMap<String, String>,
        headings: BTreeMap<String, String>,
    ) -> Self {
        let mut by_path = HashMap::new();
        let mut by_stem = HashMap::new();
        for file in &files {
            by_path.entry(file.to_lowercase()).or_insert(file.clone());
            let stem = Path::new(file)
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            by_stem.entry(stem).or_insert(file.clone());
        }

        Self {
            exact: files,
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

    pub fn resolve(&self, target: &str) -> Option<String> {
        // org id: and *Heading links carry their sigil through the parser.
        if let Some(id) = target.strip_prefix("id:") {
            return self.by_id.get(id.trim()).cloned();
        }
        if let Some(heading) = target.strip_prefix('*') {
            return self.by_heading.get(&heading.trim().to_lowercase()).cloned();
        }

        for suffix in ["md", "org", "markdown", "mdx"] {
            let candidate = format!("{target}.{suffix}");
            if self.exact.contains(&candidate) {
                return Some(candidate);
            }
        }
        if self.exact.contains(target) {
            return Some(target.to_string());
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

/// Build the link graph.
///
/// Pass one reads each file exactly once, in parallel, collecting the links it
/// declares and the aliases it answers to. Aliases must be known before
/// anything is resolved, since `[[an alias]]` points at the note declaring it.
pub fn build_link_graph(config: &Config) -> LinkGraph {
    let paths = all_notes(config);

    struct Parsed {
        relative: String,
        links: Vec<String>,
        aliases: Vec<String>,
        ids: Vec<String>,
        headings: Vec<String>,
    }

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

    let resolver = LinkResolver::new(files.clone(), aliases, ids, headings);

    let mut graph = LinkGraph {
        files,
        ..Default::default()
    };

    let ignored = IgnoredLinks::new(&config.ignore_links);

    for entry in &parsed {
        for target in &entry.links {
            match resolver.resolve(target) {
                Some(resolved) if graph.files.contains(&resolved) => {
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
}
