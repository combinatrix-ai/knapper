//! Reading a note: frontmatter, body, and everything derived from them.

use std::collections::BTreeMap;
use std::path::Path;

use crate::parser;

#[derive(Debug, Default, Clone)]
pub struct Note {
    pub path: String,
    pub title: String,
    pub content: String,
    pub frontmatter: serde_yaml::Mapping,
    pub links: Vec<String>,
    pub tags: Vec<String>,
    pub inline_fields: BTreeMap<String, String>,
    pub aliases: Vec<String>,
}

/// Split YAML frontmatter from the body.
///
/// A header that does not parse leaves the note readable: the body is used
/// as-is and the frontmatter is empty. Real vaults collect files like this,
/// and one of them must not take down a whole-vault command.
pub fn split_frontmatter(content: &str) -> (serde_yaml::Mapping, &str) {
    let empty = serde_yaml::Mapping::new();

    let Some(rest) = content.strip_prefix("---") else {
        return (empty, content);
    };
    let Some(rest) = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))
    else {
        return (empty, content);
    };
    let Some(end) = rest.find("\n---") else {
        return (empty, content);
    };

    let header = &rest[..end];
    let body = rest[end + 4..].trim_start_matches(['\r', '\n']);

    match serde_yaml::from_str::<serde_yaml::Value>(header) {
        Ok(serde_yaml::Value::Mapping(map)) => (map, body),
        _ => (empty, body),
    }
}

fn frontmatter_tags(metadata: &serde_yaml::Mapping) -> Vec<String> {
    match metadata.get(serde_yaml::Value::String("tags".into())) {
        Some(serde_yaml::Value::String(s)) => vec![s.to_string()],
        Some(serde_yaml::Value::Sequence(items)) => items
            .iter()
            .filter_map(|i| match i {
                serde_yaml::Value::String(s) => Some(s.to_string()),
                other => other.as_i64().map(|n| n.to_string()),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Parse a note, dispatching on extension so every caller sees one shape.
pub fn parse_note(path: &Path, content: &str) -> Note {
    if path.extension().and_then(|e| e.to_str()) == Some("org") {
        return crate::org::parse_org_note(path, content);
    }

    let (frontmatter, body) = split_frontmatter(content);
    let prose = parser::mask_noncontent(body);
    let inline_fields = parser::extract_inline_fields(&prose);

    let mut links = parser::extract_links(&prose);
    links.extend(parser::extract_frontmatter_links(&frontmatter));
    links.extend(parser::extract_inline_field_links(&inline_fields));

    let mut tags = frontmatter_tags(&frontmatter);
    tags.extend(parser::extract_tags(&prose));
    tags.sort();
    tags.dedup();

    let mut aliases = parser::extract_aliases(&frontmatter);
    // Logseq writes aliases as an inline property rather than in YAML.
    for key in ["alias", "aliases"] {
        if let Some(value) = inline_fields.get(key) {
            aliases.extend(
                value
                    .split(',')
                    .map(|a| a.trim().to_string())
                    .filter(|a| !a.is_empty()),
            );
        }
    }

    Note {
        path: path.to_string_lossy().into_owned(),
        title: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        content: body.to_string(),
        frontmatter,
        links,
        tags,
        inline_fields,
        aliases,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(content: &str) -> Note {
        parse_note(Path::new("n.md"), content)
    }

    /// A broken YAML header must not make a note invisible, or crash a scan.
    /// Real vaults collect files like this.
    #[test]
    fn malformed_frontmatter_leaves_the_body_readable() {
        for header in [
            "---\ntitle: \u{2}broken\n---\n", // control character
            "---\nkey: value: nested\n---\n", // unquoted colon
            "---\n\tbad indent\n---\n",       // tab indentation
        ] {
            let parsed = note(&format!("{header}\nLinks to [[Other]] and has #atag\n"));
            assert!(parsed.frontmatter.is_empty(), "header: {header:?}");
            assert!(
                parsed.links.contains(&"Other".to_string()),
                "header: {header:?}"
            );
            assert!(
                parsed.tags.contains(&"atag".to_string()),
                "header: {header:?}"
            );
        }
    }

    #[test]
    fn valid_frontmatter_is_unaffected() {
        let parsed = note("---\ntags: [alpha]\nstatus: open\n---\n\n[[Other]]\n");
        assert_eq!(
            parsed
                .frontmatter
                .get(serde_yaml::Value::String("status".into())),
            Some(&serde_yaml::Value::String("open".into()))
        );
        assert!(parsed.tags.contains(&"alpha".to_string()));
        assert_eq!(parsed.links, ["Other"]);
    }

    #[test]
    fn a_note_without_frontmatter_keeps_its_whole_body() {
        let (frontmatter, body) = split_frontmatter("# Title\n\ntext\n");
        assert!(frontmatter.is_empty());
        assert_eq!(body, "# Title\n\ntext\n");
    }

    #[test]
    fn body_links_and_property_links_both_land() {
        assert_eq!(
            note("---\nrelated: \"[[Other]]\"\n---\n\n[[In Body]]\n").links,
            ["In Body", "Other"]
        );
    }

    /// Logseq page properties are the same syntax as Dataview inline fields,
    /// so they are read without a Logseq-specific path.
    #[test]
    fn logseq_page_properties_are_read() {
        let parsed = note("title:: Knapper\ntags:: tooling, cli\nid:: 663f0a11\n\n- a block\n");
        assert_eq!(
            parsed.inline_fields.get("title").map(String::as_str),
            Some("Knapper")
        );
        assert_eq!(
            parsed.inline_fields.get("tags").map(String::as_str),
            Some("tooling, cli")
        );
        assert_eq!(
            parsed.inline_fields.get("id").map(String::as_str),
            Some("663f0a11")
        );
    }

    /// `alias::` is Logseq's equivalent of Obsidian's `aliases:` property.
    #[test]
    fn a_logseq_alias_property_becomes_an_alias() {
        let parsed = note("title:: Knapper\nalias:: knap, the tool\n\n- body\n");
        assert!(parsed.aliases.contains(&"knap".to_string()));
        assert!(parsed.aliases.contains(&"the tool".to_string()));
    }
}
