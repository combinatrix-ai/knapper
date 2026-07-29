//! org-mode parsing.
//!
//! org shares almost no syntax with markdown, so it gets its own reader.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::note::Note;

static ORG_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]\[]+?)\](?:\[[^\]]*\])?\]").unwrap());
static ORG_KEYWORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*#\+([A-Za-z_]+):\s*(.*)$").unwrap());
static ORG_HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^(\*+)[ \t]+(?:([A-Z][A-Z0-9_-]*)[ \t]+)?(?:\[#([A-Z])\][ \t]+)?(.*?)(?:[ \t]+(:(?:[\w@#%]+:)+))?[ \t]*$",
    )
    .unwrap()
});
static ORG_DRAWER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?ms)^[ \t]*:PROPERTIES:[ \t]*$(.*?)^[ \t]*:END:[ \t]*$").unwrap()
});
static ORG_PROPERTY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[ \t]*:([A-Za-z_][A-Za-z0-9_+-]*):[ \t]*(.*)$").unwrap());
static ORG_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?msi)^[ \t]*#\+BEGIN_(\w+).*?^[ \t]*#\+END_\w+[ \t]*$").unwrap()
});
static ORG_VERBATIM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[=~][^=~\n]+[=~]").unwrap());
static ORG_PLANNING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(SCHEDULED|DEADLINE|CLOSED):[ \t]*[<\[](\d{4}-\d{2}-\d{2})[^>\]]*[>\]]").unwrap()
});
static EXTERNAL_SCHEME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(https?|ftp|mailto|news|doi|elisp|shell|info|man):").unwrap()
});

pub const ORG_DONE_STATES: &[&str] = &["DONE", "CANCELLED", "CANCELED"];
pub const ORG_OPEN_STATES: &[&str] = &[
    "TODO",
    "NEXT",
    "STARTED",
    "WAITING",
    "HOLD",
    "SOMEDAY",
    "INPROGRESS",
];

#[derive(Debug, Clone, Default)]
pub struct OrgHeading {
    pub level: usize,
    pub text: String,
    pub todo: Option<String>,
    pub priority: Option<String>,
    pub tags: Vec<String>,
    pub line: usize,
}

#[derive(Debug, Default)]
pub struct OrgDocument {
    pub metadata: serde_yaml::Mapping,
    pub links: Vec<String>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub ids: Vec<String>,
    pub headings: Vec<OrgHeading>,
}

fn blank_out(text: &str) -> String {
    text.chars()
        .map(|c| if c == '\n' { '\n' } else { ' ' })
        .collect()
}

fn mask_with(text: &str, re: &Regex) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in re.find_iter(text) {
        out.push_str(&text[last..m.start()]);
        out.push_str(&blank_out(m.as_str()));
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

pub fn mask_org_noncontent(text: &str) -> String {
    mask_with(&mask_with(text, &ORG_BLOCK), &ORG_VERBATIM)
}

/// Normalize an org link target. Heading and id links keep their sigil so the
/// resolver can tell them apart from filenames.
pub fn normalize_org_target(raw: &str) -> Option<String> {
    let mut target = raw.trim().to_string();
    if target.is_empty() || EXTERNAL_SCHEME.is_match(&target) {
        return None;
    }

    if let Some(rest) = target.strip_prefix("file:") {
        // file:notes.org::*Heading -- the file is the part that resolves
        target = rest.split("::").next().unwrap_or("").to_string();
    } else if target.starts_with("id:") || target.starts_with('*') {
        return Some(target);
    } else if target.starts_with('#') {
        return None;
    }

    let mut target = target.trim().to_string();
    while let Some(rest) = target.strip_prefix("./") {
        target = rest.to_string();
    }
    if target.to_ascii_lowercase().ends_with(".org") {
        target.truncate(target.len() - 4);
    }

    (!target.is_empty()).then_some(target)
}

pub fn extract_org_links(content: &str) -> Vec<String> {
    ORG_LINK
        .captures_iter(&mask_org_noncontent(content))
        .filter_map(|c| normalize_org_target(&c[1]))
        .collect()
}

fn split_org_tags(raw: &str) -> Vec<String> {
    raw.trim_matches(':')
        .split(':')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

fn quoted_or_bare(value: &str) -> Vec<String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""([^"]+)"|(\S+)"#).unwrap());
    RE.captures_iter(value)
        .filter_map(|c| {
            c.get(1)
                .or_else(|| c.get(2))
                .map(|m| m.as_str().trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn parse_org(content: &str) -> OrgDocument {
    let mut doc = OrgDocument::default();

    for c in ORG_KEYWORD.captures_iter(content) {
        let key = c[1].to_ascii_lowercase();
        let value = c[2].trim().to_string();
        match key.as_str() {
            "filetags" => doc.tags.extend(split_org_tags(&value)),
            "alias" | "aliases" | "roam_alias" | "roam_aliases" => {
                doc.aliases.extend(quoted_or_bare(&value))
            }
            _ => {
                doc.metadata.insert(
                    serde_yaml::Value::String(key),
                    serde_yaml::Value::String(value),
                );
            }
        }
    }

    // The first drawer is file-level metadata, which is where org-roam keeps
    // a note's ID.
    for (index, drawer) in ORG_DRAWER.captures_iter(content).enumerate() {
        for p in ORG_PROPERTY.captures_iter(&drawer[1]) {
            let key = p[1].to_ascii_lowercase();
            let value = p[2].trim().to_string();
            match key.as_str() {
                "id" => doc.ids.push(value),
                "roam_aliases" | "roam_alias" => doc.aliases.extend(quoted_or_bare(&value)),
                _ if index == 0 => {
                    doc.metadata.insert(
                        serde_yaml::Value::String(key),
                        serde_yaml::Value::String(value),
                    );
                }
                _ => {}
            }
        }
    }

    for c in ORG_HEADING.captures_iter(content) {
        let tags = c
            .get(5)
            .map(|m| split_org_tags(m.as_str()))
            .unwrap_or_default();
        doc.tags.extend(tags.clone());
        doc.headings.push(OrgHeading {
            level: c[1].len(),
            text: c
                .get(4)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default(),
            todo: c.get(2).map(|m| m.as_str().to_string()),
            priority: c.get(3).map(|m| m.as_str().to_string()),
            tags,
            line: content[..c.get(0).unwrap().start()].matches('\n').count() + 1,
        });
    }

    doc.links = extract_org_links(content);
    doc.tags.sort();
    doc.tags.dedup();
    doc.aliases.sort();
    doc.aliases.dedup();
    doc
}

/// SCHEDULED / DEADLINE / CLOSED dates from an org subtree.
pub fn parse_org_planning(text: &str) -> Vec<(String, String)> {
    ORG_PLANNING
        .captures_iter(text)
        .map(|c| (c[1].to_ascii_lowercase(), c[2].to_string()))
        .collect()
}

/// Read an org file into the same shape a markdown note produces.
pub fn parse_org_note(path: &Path, content: &str) -> Note {
    let doc = parse_org(content);

    let title = doc
        .metadata
        .get(serde_yaml::Value::String("title".into()))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });

    Note {
        path: path.to_string_lossy().into_owned(),
        title,
        content: content.to_string(),
        frontmatter: doc.metadata,
        links: doc.links,
        tags: doc.tags,
        inline_fields: Default::default(),
        aliases: doc.aliases,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTENT: &str = r#"#+TITLE: Thesis
#+FILETAGS: :research:project:
#+ROAM_ALIASES: "Doctoral Thesis" Diss

* Overview
  :PROPERTIES:
  :ID:       aq7b2m
  :STATUS:   open
  :END:

Links to [[file:lit-review.org][Lit Review]].

** TODO [#A] draft intro                                             :writing:
   DEADLINE: <2026-08-01 Sat>
   SCHEDULED: <2026-07-30 Thu>
** DONE outline
   CLOSED: [2026-07-20 Mon]
"#;

    fn meta(doc: &OrgDocument, key: &str) -> Option<String> {
        doc.metadata
            .get(serde_yaml::Value::String(key.into()))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    #[test]
    fn link_targets_normalise_to_a_resolvable_name() {
        for (raw, expected) in [
            ("file:notes.org", Some("notes")),
            ("file:sub/notes.org", Some("sub/notes")),
            ("./notes.org", Some("notes")),
            ("notes.org", Some("notes")),
            ("file:notes.org::*Heading", Some("notes")),
            // Heading and id links keep their sigil so the resolver can tell
            // them apart from filenames.
            ("id:aq7b2m", Some("id:aq7b2m")),
            ("*Some Heading", Some("*Some Heading")),
            ("https://example.com", None),
            ("mailto:a@b.com", None),
            ("#custom-id", None),
        ] {
            assert_eq!(
                normalize_org_target(raw).as_deref(),
                expected,
                "input: {raw}"
            );
        }
    }

    #[test]
    fn a_link_description_is_display_only() {
        assert_eq!(extract_org_links("[[file:a.org][Some Description]]"), ["a"]);
        assert_eq!(extract_org_links("[[file:a.org]]"), ["a"]);
    }

    #[test]
    fn src_blocks_and_verbatim_are_not_scanned() {
        let content = "#+BEGIN_SRC python\n[[file:hidden.org]]\n#+END_SRC\n\n[[file:real.org]]\n";
        assert_eq!(extract_org_links(content), ["real"]);
        assert_eq!(
            extract_org_links("=[[file:hidden.org]]= [[file:real.org]]"),
            ["real"]
        );
    }

    #[test]
    fn keywords_become_metadata() {
        assert_eq!(
            meta(&parse_org(CONTENT), "title").as_deref(),
            Some("Thesis")
        );
    }

    #[test]
    fn filetags_and_heading_tags_both_count() {
        let mut expected = vec!["project", "research", "writing"];
        expected.sort();
        assert_eq!(parse_org(CONTENT).tags, expected);
    }

    #[test]
    fn roam_aliases_read_quoted_and_bare_values() {
        let mut expected = vec!["Diss", "Doctoral Thesis"];
        expected.sort();
        assert_eq!(parse_org(CONTENT).aliases, expected);
    }

    /// The first drawer is file-level metadata, which is where org-roam keeps
    /// a note's ID.
    #[test]
    fn the_first_drawer_is_file_metadata() {
        let doc = parse_org(CONTENT);
        assert_eq!(doc.ids, ["aq7b2m"]);
        assert_eq!(meta(&doc, "status").as_deref(), Some("open"));
    }

    #[test]
    fn headings_carry_state_priority_and_level() {
        let doc = parse_org(CONTENT);
        let find = |text: &str| {
            doc.headings
                .iter()
                .find(|h| h.text == text)
                .unwrap_or_else(|| panic!("no heading {text:?}"))
        };

        let intro = find("draft intro");
        assert_eq!(intro.todo.as_deref(), Some("TODO"));
        assert_eq!(intro.priority.as_deref(), Some("A"));
        assert_eq!(intro.level, 2);
        assert_eq!(find("outline").todo.as_deref(), Some("DONE"));
        assert_eq!(find("Overview").todo, None);
    }

    #[test]
    fn planning_timestamps_are_read() {
        let planning = parse_org_planning(CONTENT);
        let get = |key: &str| {
            planning
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("deadline"), Some("2026-08-01"));
        assert_eq!(get("scheduled"), Some("2026-07-30"));
        assert_eq!(get("closed"), Some("2026-07-20"));
    }

    #[test]
    fn an_org_note_takes_its_title_from_the_keyword() {
        let note = parse_org_note(std::path::Path::new("thesis.org"), CONTENT);
        assert_eq!(note.title, "Thesis");
        assert_eq!(note.links, ["lit-review"]);
    }

    /// Without `#+TITLE:`, the filename is the title -- the same rule
    /// markdown notes follow.
    #[test]
    fn an_org_note_without_a_title_keyword_falls_back_to_its_filename() {
        let note = parse_org_note(std::path::Path::new("sub/notes.org"), "* Heading\n");
        assert_eq!(note.title, "notes");
    }
}
