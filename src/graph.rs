//! The link graph, and the resolver that turns a link target into a path.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use rayon::prelude::*;

use crate::note::parse_note;
use crate::org;
use crate::vault::{all_notes, is_org, relative_path, Config};

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
