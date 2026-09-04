//! Link and tag extraction.
//!
//! A port of the Python `knapper.parser`. The behaviour it implements is
//! pinned by `tests/contract/cases.yaml`, which both implementations answer
//! to, so this file is a translation of a specification rather than a
//! reinterpretation.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use percent_encoding::percent_decode_str;
use regex::Regex;

// [[target]], [[target|alias]], [[target#heading]], [[target^block]]
static WIKILINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]|]+?)(?:\|[^\]]*)?\]\]").unwrap());

// [text](target). Images are excluded by checking the preceding byte, since
// the regex crate has no lookbehind.
//
// A target wrapped in <> may hold spaces, so it cannot be read with the same
// pattern as a bare one: `<With Space.md>` matched up to the space and left
// the graph pointing at `With`, which lost the note an inbound link and
// invented a broken one. What a bare target may contain comes from
// `links::DESTINATION`, so the graph and the rewriters cannot disagree about
// it again.
static MARKDOWN_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"\[[^\]]*\]\(\s*(?:<([^>]*)>|({}))((?:[ \t]+"[^"]*")?)[ \t]*\)"#,
        crate::links::DESTINATION
    ))
    .unwrap()
});

// Unicode-aware so CJK tags are found; nested tags keep their full path.
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)#([\w/-]+)").unwrap());

static EXTERNAL_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z][a-zA-Z0-9+.\-]*:").unwrap());

// Regions whose contents are not references.
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]+`").unwrap());
static OBSIDIAN_COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)%%.*?%%").unwrap());
static OUTLINER_MACRO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\{\{.*?\}\}").unwrap());
static BLOCK_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(\([^)\n]+\)\)").unwrap());

// Markdown headings and Obsidian block IDs are indexed from the masked body,
// so links in code fences, inline code, and comments never become anchors.
static ATX_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[ \t]{0,3}#{1,6}(?:[ \t]+(.*?)[ \t]*|[ \t]*)$").unwrap());
static SETEXT_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[ \t]{0,3}(?:=+|-+)[ \t]*$").unwrap());
static BLOCK_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[ \t])\^([A-Za-z0-9_-]+)[ \t]*$").unwrap());

// Dataview inline fields, which are also Logseq properties.
static INLINE_FIELD_BRACKETED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\[(]\s*([A-Za-z_][\w-]*)\s*::\s*((?:[^\])]|\]\])*)[\])]").unwrap()
});
static INLINE_FIELD_BARE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*(?:[-*+]\s+)?([A-Za-z_][\w-]*)::[ \t]+(.*)$").unwrap()
});

pub const NOTE_SUFFIXES: &[&str] = &["md", "markdown", "mdx"];

/// Attachments, as a deny list rather than an allow list, so a dotted note
/// name such as `proj.knapper.design` still resolves.
pub const ATTACHMENT_SUFFIXES: &[&str] = &[
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "svg",
    "bmp",
    "ico",
    "avif",
    "pdf",
    "doc",
    "docx",
    "xls",
    "xlsx",
    "ppt",
    "pptx",
    "mp3",
    "wav",
    "m4a",
    "ogg",
    "flac",
    "mp4",
    "mov",
    "avi",
    "mkv",
    "webm",
    "zip",
    "tar",
    "gz",
    "7z",
    "rar",
    "canvas",
    "base",
    "excalidraw",
];

/// Replace a span with spaces, keeping newlines so offsets and line numbers
/// survive masking.
///
/// A masked character is replaced by as many spaces as it had bytes, so the
/// masked text is byte-for-byte the same length as the original. That is what
/// lets a rewriter find a span in the masked text and edit that same span in
/// the file: with one space per *character*, every offset after a masked CJK
/// comment would be wrong.
fn blank_out(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\n' {
            out.push('\n');
        } else {
            out.extend(std::iter::repeat(' ').take(c.len_utf8()));
        }
    }
    out
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

/// Blank out fenced code blocks, line by line.
///
/// This is a scanner rather than a regex because a fence must be closed by the
/// same marker that opened it, and the regex crate has no backreferences.
/// Approximating that with an alternation lets a ``` block be closed by a ~~~
/// line, which leaks whatever follows into the link graph.
///
/// Only a *closed* block is masked. CommonMark runs an unclosed fence to the
/// end of the document, which is right for rendering, but knapper reports what
/// is in a vault: one stray ``` should not silently delete every link, tag and
/// heading written after it. An opening fence with no closing one is read as
/// ordinary content.
fn mask_fenced_code(text: &str) -> String {
    // A fence must start at column zero. An indented ``` inside a list is
    // part of the list item's content, and treating it as a fence swallows
    // everything after it.
    fn fence_marker(line: &str) -> Option<&str> {
        for marker in ["```", "~~~"] {
            if line.starts_with(marker) {
                let first = marker.as_bytes()[0] as char;
                let run = line.chars().take_while(|c| *c == first).count();
                return Some(&line[..run]);
            }
        }
        None
    }

    // Which lines to blank is decided before any are written, because an
    // opening fence is only known to be a fence once its close is found.
    let lines: Vec<&str> = text.split('\n').collect();
    let mut masked = vec![false; lines.len()];
    let mut open: Option<(usize, &str)> = None;

    for (index, line) in lines.iter().enumerate() {
        match open {
            None => {
                if let Some(marker) = fence_marker(line) {
                    open = Some((index, marker));
                }
            }
            Some((start, marker)) => {
                // A closing fence is the same character, at least as long.
                let closes = fence_marker(line)
                    .map(|m| m.starts_with(&marker[..1]) && m.len() >= marker.len())
                    .unwrap_or(false);
                if closes {
                    for flag in masked[start..=index].iter_mut() {
                        *flag = true;
                    }
                    open = None;
                }
            }
        }
    }

    let mut out = String::with_capacity(text.len());
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if masked[index] {
            out.push_str(&blank_out(line));
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Blank out code, comments, outliner macros and block references.
///
/// Each pattern is guarded by a substring test, which is far cheaper than
/// running it, and most notes contain none of them.
pub fn mask_noncontent(text: &str) -> String {
    let mut out = std::borrow::Cow::Borrowed(text);
    if out.contains("```") || out.contains("~~~") {
        out = mask_fenced_code(&out).into();
    }
    if out.contains("%%") {
        out = mask_with(&out, &OBSIDIAN_COMMENT).into();
    }
    if out.contains('`') {
        out = mask_with(&out, &INLINE_CODE).into();
    }
    if out.contains("{{") {
        out = mask_with(&out, &OUTLINER_MACRO).into();
    }
    if out.contains("((") {
        out = mask_with(&out, &BLOCK_REF).into();
    }
    out.into_owned()
}

/// Drop a `#heading` or `^block-id` suffix from a link target.
pub fn strip_anchor(target: &str) -> String {
    let mut result = target;
    for sep in ['#', '^'] {
        if let Some(idx) = result.find(sep) {
            if idx > 0 {
                result = &result[..idx];
            }
        }
    }
    result.trim().to_string()
}

/// The resolvable target of a wikilink, given the text between its brackets.
///
/// `None` means the brackets held nothing to resolve.
pub fn normalize_wikilink_target(inner: &str) -> Option<String> {
    // `[[#Heading]]` and `[[^block-id]]` navigate within the current note.
    // They name no other note, so the path portion normalizes to `None`; the
    // occurrence scanner keeps their anchor for validation.
    let inner = inner.trim();
    if inner.starts_with('#') || inner.starts_with('^') {
        return None;
    }
    // Roam exports write a link wrapped in a link, [[[[Ideas]]]];
    // the extra brackets are not part of the name.
    let target = strip_anchor(inner);
    let target = target.trim_matches(['[', ']']).trim().to_string();
    (!target.is_empty()).then_some(target)
}

/// The kind of an Obsidian/Markdown anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorKind {
    Heading(String),
    Block(String),
}

/// Classify the anchor portion kept by `links::RawLink`.
///
/// Obsidian accepts both `^id` and `#^id` for block references. A normal
/// `#text` reference is a heading. Empty anchors are unknown and therefore
/// cannot accidentally validate.
pub fn anchor_kind(anchor: &str) -> Option<AnchorKind> {
    let decoded = percent_decode_str(anchor).decode_utf8_lossy();
    let anchor = decoded.trim();
    let anchor = anchor.strip_prefix('#').unwrap_or(anchor);
    if let Some(id) = anchor.strip_prefix('^') {
        let id = id.trim();
        return (!id.is_empty()).then(|| AnchorKind::Block(id.to_lowercase()));
    }
    let heading = anchor.trim();
    (!heading.is_empty()).then(|| AnchorKind::Heading(normalize_heading(heading)))
}

/// The anchors declared by one Markdown note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnchorInventory {
    pub headings: BTreeSet<String>,
    pub blocks: BTreeSet<String>,
}

impl AnchorInventory {
    pub fn contains(&self, anchor: &str) -> bool {
        match anchor_kind(anchor) {
            Some(AnchorKind::Heading(heading)) => self.headings.contains(&heading),
            Some(AnchorKind::Block(id)) => self.blocks.contains(&id),
            None => false,
        }
    }
}

/// Normalize the visible text of a Markdown heading for Obsidian-style
/// anchor matching. Closing `#` decoration is removed only when separated by
/// whitespace, so a heading such as `C#` remains unchanged.
pub fn normalize_heading(text: &str) -> String {
    let trimmed = text.trim();
    let mut suffix_start = trimmed.len();
    while suffix_start > 0 {
        let (index, ch) = trimmed[..suffix_start].char_indices().next_back().unwrap();
        if ch == '#' {
            suffix_start = index;
        } else {
            break;
        }
    }
    let normalized = if suffix_start < trimmed.len()
        && trimmed[..suffix_start]
            .chars()
            .last()
            .is_some_and(char::is_whitespace)
    {
        trimmed[..suffix_start].trim_end()
    } else {
        trimmed
    };
    normalized.to_lowercase()
}

/// Extract Markdown headings and block IDs from a note.
///
/// Frontmatter is removed before masking. The same content mask used by link
/// and tag parsing then excludes closed code fences and Obsidian comments,
/// while preserving line structure for setext headings and block IDs.
pub fn extract_anchor_inventory(content: &str) -> AnchorInventory {
    let (_, body) = crate::note::split_frontmatter(content);
    let masked = mask_noncontent(body);
    let lines: Vec<&str> = masked.split('\n').collect();
    let mut inventory = AnchorInventory::default();

    for (index, line) in lines.iter().enumerate() {
        if let Some(captures) = ATX_HEADING.captures(line) {
            if let Some(text) = captures
                .get(1)
                .map(|m| m.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                inventory.headings.insert(normalize_heading(text));
            }
        }

        // Setext headings are a non-empty prose line immediately followed by
        // `===` or `---`. The frontmatter delimiter is already outside body.
        if index + 1 < lines.len()
            && SETEXT_HEADING.is_match(lines[index + 1])
            && !line.trim().is_empty()
        {
            inventory.headings.insert(normalize_heading(line));
        }

        if let Some(captures) = BLOCK_ID.captures(line) {
            if let Some(id) = captures.get(1) {
                inventory.blocks.insert(id.as_str().to_lowercase());
            }
        }
    }

    inventory
}

/// The resolvable target of a markdown href that is already percent-decoded
/// and has already had its anchor removed.
///
/// `None` means the href is not a note reference: an attachment, or nothing at
/// all. This is where "a dot that is not an extension is part of the name"
/// lives, so a Dendron `proj.knapper.design` survives and a `.pdf` does not
/// become a link.
pub fn normalize_markdown_path(target: &str) -> Option<String> {
    let mut target = target.to_string();
    if target.is_empty() {
        return None;
    }

    // Strip a note extension; skip attachments. A dot that is neither is
    // part of the name.
    if let Some((stem, ext)) = target.rsplit_once('.') {
        let ext = ext.to_ascii_lowercase();
        if NOTE_SUFFIXES.contains(&ext.as_str()) {
            target = stem.to_string();
        } else if ATTACHMENT_SUFFIXES.contains(&ext.as_str()) {
            return None;
        }
    }

    while let Some(rest) = target.strip_prefix("./") {
        target = rest.to_string();
    }

    (!target.is_empty()).then_some(target)
}

/// The resolvable target of a markdown href, as written.
pub fn normalize_markdown_target(raw: &str) -> Option<String> {
    if EXTERNAL_SCHEME.is_match(raw) || raw.starts_with('#') || raw.starts_with("//") {
        return None;
    }
    let decoded = percent_decode_str(raw).decode_utf8_lossy().to_string();
    normalize_markdown_path(&strip_anchor(&decoded))
}

pub fn extract_wikilinks(content: &str) -> Vec<String> {
    WIKILINK
        .captures_iter(content)
        .filter_map(|c| normalize_wikilink_target(&c[1]))
        .collect()
}

pub fn extract_markdown_links(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let mut targets = Vec::new();

    for c in MARKDOWN_LINK.captures_iter(content) {
        let whole = c.get(0).unwrap();
        // No lookbehind in this regex engine, so check for the image "!".
        if whole.start() > 0 && bytes[whole.start() - 1] == b'!' {
            continue;
        }
        let href = c.get(1).or_else(|| c.get(2)).unwrap().as_str();
        targets.extend(normalize_markdown_target(href));
    }
    targets
}

pub fn extract_links(content: &str) -> Vec<String> {
    let mut links = extract_wikilinks(content);
    links.extend(extract_markdown_links(content));
    links
}

pub fn extract_tags(content: &str) -> Vec<String> {
    TAG.captures_iter(content)
        .filter_map(|c| {
            let tag = c[1].trim_end_matches('/');
            // Purely numeric tokens are dates like #8/18, not tags.
            (!tag.is_empty() && tag.chars().any(|ch| ch.is_alphabetic())).then(|| tag.to_string())
        })
        .collect()
}

/// Dataview inline fields, which are also Logseq page properties.
pub fn extract_inline_fields(content: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    // Both patterns need "::" to match at all, and most notes have none.
    if !content.contains("::") {
        return fields;
    }
    for re in [&*INLINE_FIELD_BRACKETED, &*INLINE_FIELD_BARE] {
        for c in re.captures_iter(content) {
            fields
                .entry(c[1].trim().to_string())
                .or_insert_with(|| c[2].trim().to_string());
        }
    }
    fields
}

pub fn extract_inline_field_links(fields: &BTreeMap<String, String>) -> Vec<String> {
    fields.values().flat_map(|v| extract_wikilinks(v)).collect()
}

/// Wikilinks written as frontmatter property values. Obsidian treats these as
/// real links, and they are the portable way to write a typed relation.
pub fn extract_frontmatter_links(metadata: &serde_yaml::Mapping) -> Vec<String> {
    fn walk(value: &serde_yaml::Value, out: &mut Vec<String>) {
        match value {
            serde_yaml::Value::String(s) => out.extend(extract_wikilinks(s)),
            serde_yaml::Value::Sequence(items) => items.iter().for_each(|i| walk(i, out)),
            serde_yaml::Value::Mapping(map) => map.values().for_each(|v| walk(v, out)),
            _ => {}
        }
    }

    let mut out = Vec::new();
    for (key, value) in metadata {
        if key.as_str() == Some("tags") {
            continue;
        }
        walk(value, &mut out);
    }
    out
}

/// The `aliases` (or `alias`) property. Obsidian resolves `[[an alias]]` to
/// the note declaring it, so link resolution needs these.
pub fn extract_aliases(metadata: &serde_yaml::Mapping) -> Vec<String> {
    let raw = metadata
        .get(serde_yaml::Value::String("aliases".into()))
        .or_else(|| metadata.get(serde_yaml::Value::String("alias".into())));

    match raw {
        Some(serde_yaml::Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
        Some(serde_yaml::Value::Sequence(items)) => items
            .iter()
            .filter_map(|i| i.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(src: &str) -> serde_yaml::Mapping {
        serde_yaml::from_str(src).unwrap()
    }

    /// knapper is not tied to Obsidian, so both link syntaxes must resolve:
    /// the wikilinks used by Obsidian/Foam/Dendron, and the inline markdown
    /// links used by everything else.
    #[test]
    fn wikilinks_drop_alias_heading_and_block_id() {
        for (content, expected) in [
            ("[[Note]]", vec!["Note"]),
            ("[[Note A]]", vec!["Note A"]),
            ("[[Note|alias]]", vec!["Note"]),
            ("[[folder/Note]]", vec!["folder/Note"]),
            ("[[Note#heading]]", vec!["Note"]),
            ("[[Note^block-id]]", vec!["Note"]),
            ("[[Note#heading|alias]]", vec!["Note"]),
            ("[[#heading]]", vec![]),
            ("[[#heading|label]]", vec![]),
            ("[[^block-id]]", vec![]),
            ("[[A]] and [[B]]", vec!["A", "B"]),
            ("no links here", vec![]),
        ] {
            assert_eq!(extract_wikilinks(content), expected, "input: {content}");
        }
    }

    #[test]
    fn extracts_real_markdown_anchors_only() {
        let inventory = extract_anchor_inventory(
            "---\nheading: '# Not frontmatter'\n---\n\n\
             # Main Heading ###\n\n\
             Setext Heading\n--------------\n\n\
             A paragraph. ^Block_ID\n\n\
             ```md\n# Not code\ntext ^not-code\n```\n\n\
             %% ## Not comment\ntext ^not-comment %%\n",
        );

        assert!(inventory.contains("#main heading"));
        assert!(inventory.contains("#Main Heading ###"));
        assert!(inventory.contains("#Setext Heading"));
        assert!(inventory.contains("#Setext%20Heading"));
        assert!(inventory.contains("#^block_id"));
        assert!(!inventory.contains("#Not frontmatter"));
        assert!(!inventory.contains("#Not code"));
        assert!(!inventory.contains("^not-code"));
        assert!(!inventory.contains("#Not comment"));
        assert!(!inventory.contains("^not-comment"));
    }

    #[test]
    fn markdown_links_are_normalised() {
        for (content, expected) in [
            ("[text](Note.md)", "Note"),
            ("[text](folder/Note.md)", "folder/Note"),
            ("[text](./Note.md)", "Note"),
            ("[text](My%20Note.md)", "My Note"),
            ("[text](Note.md#heading)", "Note"),
            ("[text](Note.md \"title\")", "Note"),
            ("[text](<Note.md>)", "Note"),
        ] {
            assert_eq!(
                extract_markdown_links(content),
                vec![expected],
                "input: {content}"
            );
        }
    }

    #[test]
    fn external_targets_and_attachments_are_not_links() {
        for content in [
            "[text](https://example.com)",
            "[text](http://example.com/page.md)",
            "[mail](mailto:a@b.com)",
            "[obs](obsidian://open?vault=x)",
            "![image](picture.png)",
            // An embed is not an outgoing link.
            "![image](picture.md)",
            "[doc](report.pdf)",
            "[anchor](#section)",
            "[proto](//cdn.example.com/x.md)",
        ] {
            assert!(
                extract_markdown_links(content).is_empty(),
                "should not link: {content}"
            );
        }
    }

    #[test]
    fn both_syntaxes_resolve_in_one_note() {
        let content = "See [[Wiki Note]] and [inline](folder/Other.md) plus [ext](https://x.com).";
        assert_eq!(extract_links(content), ["Wiki Note", "folder/Other"]);
    }

    #[test]
    fn tags_are_unicode_aware_and_nest() {
        for (content, expected) in [
            ("#work", "work"),
            ("text #mid text", "mid"),
            ("#has_underscore", "has_underscore"),
            ("#with-dash", "with-dash"),
            ("#parent/child", "parent/child"),
            ("#parent/child/grand", "parent/child/grand"),
            ("#日本語タグ", "日本語タグ"),
            ("#ウマ娘", "ウマ娘"),
            ("#中文标签", "中文标签"),
            ("#tag/", "tag"),
        ] {
            assert_eq!(extract_tags(content), vec![expected], "input: {content}");
        }
    }

    /// Headings and purely numeric tokens are not tags. Dates written as
    /// #8/18 show up constantly in imported social posts.
    #[test]
    fn headings_and_numeric_tokens_are_not_tags() {
        for content in ["# Heading", "## Sub", "#123", "#8/18", "#2026/07"] {
            assert!(
                extract_tags(content).is_empty(),
                "should not tag: {content}"
            );
        }
    }

    // ------------------------------------------------------------ masking --
    // Code and comments are not prose; links inside them are not references.

    #[test]
    fn fenced_code_is_not_scanned() {
        let content = "```python\n# TODO\nx = '[[Hidden]]'\n```\n\n[[Real]]\n";
        assert_eq!(extract_links(&mask_noncontent(content)), ["Real"]);
    }

    #[test]
    fn tilde_fences_mask_too() {
        assert_eq!(
            extract_links(&mask_noncontent("~~~\n[[Hidden]]\n~~~\n[[Real]]\n")),
            ["Real"]
        );
    }

    /// A note with one stray ``` in it is a typo, not a note whose second half
    /// has been retracted; its links, tags and headings are still there.
    #[test]
    fn an_unterminated_fence_is_not_a_fence() {
        let content = "```\nan example that forgot its close\n\n## Later\n[[Real]]\n#realtag\n";
        let masked = mask_noncontent(content);
        assert_eq!(extract_links(&masked), ["Real"]);
        assert_eq!(extract_tags(&masked), ["realtag"]);
        assert!(masked.contains("## Later"));
    }

    /// The other half of the same rule: a fence that is closed still masks.
    #[test]
    fn a_closed_fence_still_masks_its_content() {
        assert_eq!(
            extract_links(&mask_noncontent("```\n[[Hidden]]\n```\n[[Real]]\n")),
            ["Real"]
        );
    }

    /// A ``` that is not at column zero belongs to the list item, so it opens
    /// nothing -- and now that an unclosed fence is content, the closing fence
    /// of a *later* real block must not be paired with it either.
    #[test]
    fn an_indented_fence_does_not_open_a_block() {
        let content = "- item\n  ```\n  [[Real]]\n  ```\n[[AlsoReal]]\n";
        assert_eq!(
            extract_links(&mask_noncontent(content)),
            ["Real", "AlsoReal"]
        );
    }

    /// A block opened by ``` may be closed by a longer run of the same
    /// character, and that still counts as closed.
    #[test]
    fn a_longer_closing_marker_still_closes() {
        assert_eq!(
            extract_links(&mask_noncontent("```\n[[Hidden]]\n`````\n[[Real]]\n")),
            ["Real"]
        );
    }

    #[test]
    fn inline_code_is_masked() {
        assert_eq!(
            extract_links(&mask_noncontent("`[[Hidden]]` and [[Real]]")),
            ["Real"]
        );
    }

    #[test]
    fn obsidian_comments_are_masked() {
        assert_eq!(
            extract_links(&mask_noncontent("%% [[Hidden]] %% [[Real]]")),
            ["Real"]
        );
    }

    /// `#include` inside a code fence used to become a tag called "include".
    #[test]
    fn c_preprocessor_directives_are_not_tags() {
        let content = "```c\n#include <stdio.h>\n#define FOO 1\n```\n\n#realtag\n";
        assert_eq!(extract_tags(&mask_noncontent(content)), ["realtag"]);
    }

    /// A rewriter locates a link in the masked text and edits that offset in
    /// the original, so masking must not move a single byte.
    #[test]
    fn masking_preserves_byte_offsets() {
        for content in [
            "a\n```\n[[隠し]]\n```\nb [[R]]",
            "%% 日本語のコメント [[H]] %%\n[[R]]",
            "`インラインコード` and [[R]]",
            "{{[[クエリ]]}} [[R]]",
            "((ブロック参照)) [[R]]",
        ] {
            let masked = mask_noncontent(content);
            assert_eq!(masked.len(), content.len(), "input: {content}");
            let at = content.find("[[R]]").unwrap();
            assert_eq!(&masked[at..at + 5], "[[R]]", "input: {content}");
        }
    }

    /// Masking blanks characters in place, so anything reporting a line
    /// number still reports the right one.
    #[test]
    fn masking_preserves_line_numbers() {
        let content = "a\n```\n[[H]]\n```\nb [[R]]";
        let masked = mask_noncontent(content);
        assert_eq!(masked.lines().count(), content.lines().count());
        assert!(masked.lines().nth(4).unwrap().contains("[[R]]"));
    }

    #[test]
    fn callouts_and_footnotes_are_still_prose() {
        let content = "> [!note] T\n> see [[In Callout]]\n\n[^1]: see [[In Footnote]]\n";
        assert_eq!(
            extract_links(&mask_noncontent(content)),
            ["In Callout", "In Footnote"]
        );
    }

    // ------------------------------------------------------------ aliases --

    #[test]
    fn aliases_read_both_keys_and_both_shapes() {
        assert_eq!(
            extract_aliases(&yaml("aliases: [三井物産, Mitsui]")),
            ["三井物産", "Mitsui"]
        );
        assert_eq!(extract_aliases(&yaml("aliases: Solo")), ["Solo"]);
        assert_eq!(extract_aliases(&yaml("alias: [Old Key]")), ["Old Key"]);
        assert!(extract_aliases(&yaml("{}")).is_empty());
        assert!(extract_aliases(&yaml("aliases: null")).is_empty());
    }

    // -------------------------------------------------- frontmatter links --
    // A property whose value is a wikilink is a real link in Obsidian.

    #[test]
    fn frontmatter_links_come_from_scalars_and_lists() {
        let metadata =
            yaml("related: \"[[Other]]\"\nsources: [\"[[A]]\", \"[[B]]\"]\ntitle: no link here");
        assert_eq!(extract_frontmatter_links(&metadata), ["Other", "A", "B"]);
    }

    #[test]
    fn the_tags_property_is_never_a_link() {
        assert!(extract_frontmatter_links(&yaml("tags: [\"[[NotALink]]\"]")).is_empty());
    }

    // ------------------------------------------------------ dotted names --
    // Dendron names notes proj.a.b; ".b" is not a file extension.

    #[test]
    fn dotted_note_names_survive() {
        for (content, expected) in [
            ("[d](proj.knapper.design)", "proj.knapper.design"),
            ("[d](proj.knapper.design.md)", "proj.knapper.design"),
            ("[d](notes/v1.2.release)", "notes/v1.2.release"),
        ] {
            assert_eq!(extract_links(content), vec![expected], "input: {content}");
        }
    }

    #[test]
    fn attachments_are_still_ignored() {
        for content in [
            "[i](pic.png)",
            "[p](doc.pdf)",
            "[c](board.canvas)",
            "[z](bundle.zip)",
        ] {
            assert!(
                extract_links(content).is_empty(),
                "should not link: {content}"
            );
        }
    }

    // ------------------------------------------------- inline fields --
    // Dataview inline fields are also Logseq properties: the same syntax
    // carrying the same meaning, so both come from one implementation.

    fn fields(content: &str) -> BTreeMap<String, String> {
        extract_inline_fields(&mask_noncontent(content))
    }

    fn pairs(items: &[(&str, &str)]) -> BTreeMap<String, String> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn every_inline_field_form_is_recognised() {
        for (content, expected) in [
            ("[cal:: 542]", pairs(&[("cal", "542")])),
            ("(rating:: 4)", pairs(&[("rating", "4")])),
            ("status:: open", pairs(&[("status", "open")])),
            ("- status:: open", pairs(&[("status", "open")])),
            ("  indented:: yes", pairs(&[("indented", "yes")])),
            ("[a:: 1] and [b:: 2]", pairs(&[("a", "1"), ("b", "2")])),
            ("[with-dash:: v]", pairs(&[("with-dash", "v")])),
        ] {
            assert_eq!(fields(content), expected, "input: {content}");
        }
    }

    #[test]
    fn several_fields_share_a_line() {
        let line = "- lunch #meal [meal:: dinner] [cal:: 542] [p:: 28.8] [source:: Health/Food.md]";
        assert_eq!(
            fields(line),
            pairs(&[
                ("meal", "dinner"),
                ("cal", "542"),
                ("p", "28.8"),
                ("source", "Health/Food.md"),
            ])
        );
    }

    #[test]
    fn the_first_value_of_a_repeated_field_wins() {
        assert_eq!(
            fields("[k:: first]\n[k:: second]"),
            pairs(&[("k", "first")])
        );
    }

    #[test]
    fn code_and_bare_colons_are_not_fields() {
        for content in [
            "```cpp\nstd::cout << x;\n```",
            "see `a::b` inline",
            "namespace::member without space",
            "http://example.com/a::b",
            "mid-line bare rating:: 4 is not a field",
        ] {
            assert!(
                fields(content).is_empty(),
                "should not be a field: {content}"
            );
        }
    }

    #[test]
    fn fields_inside_a_fence_are_ignored() {
        assert_eq!(
            fields("```yaml\nkey:: value\n```\n\nreal:: yes"),
            pairs(&[("real", "yes")])
        );
    }

    /// A field whose value is a wikilink is a real link, as Dataview treats
    /// it. A bare path is not -- Dataview does not resolve those either.
    #[test]
    fn typed_links_reach_the_graph_but_bare_paths_do_not() {
        assert_eq!(
            extract_inline_field_links(&fields("[supports:: [[Some Note]]]")),
            ["Some Note"]
        );
        assert_eq!(
            extract_inline_field_links(&fields("[rel:: [[A]] and [[B]]]")),
            ["A", "B"]
        );
        assert!(extract_inline_field_links(&fields("[source:: path/to/x.md]")).is_empty());
    }

    // ----------------------------------------------------- outliners --
    // knapper reads the file-level projection of an outliner graph and
    // ignores block identity. What it must not do is let outliner syntax
    // leak into the link graph as phantom edges.

    /// `{{[[query]]}}` names a command, not a note.
    #[test]
    fn outliner_macros_hold_no_links() {
        for (content, expected) in [
            (
                "{{[[query]]: {and: [[TODO]] [[Daily Tasks]]}}}\n[[Real]]",
                "Real",
            ),
            ("- {{[[TODO]]}} do the thing [[Project]]", "Project"),
            ("- {{[[DONE]]}} finished [[Project]]", "Project"),
            ("{{embed: ((abc123))}}\n[[Real]]", "Real"),
            ("{{[[roam/js]]}}\n[[Real]]", "Real"),
        ] {
            assert_eq!(
                extract_links(&mask_noncontent(content)),
                vec![expected],
                "input: {content}"
            );
        }
    }

    /// `{{date}}` is Obsidian template syntax, and holds no links anyway.
    #[test]
    fn obsidian_core_template_syntax_is_unaffected() {
        assert_eq!(
            extract_links(&mask_noncontent("{{date}} and [[Real]]")),
            ["Real"]
        );
    }

    #[test]
    fn block_references_are_not_links() {
        for content in [
            "((GGkhSlrsZ))",
            "[*](((GGkhSlrsZ)))",
            "((663f0a11-1111-2222-3333-444455556666))",
        ] {
            assert!(
                extract_links(&mask_noncontent(content)).is_empty(),
                "should not link: {content}"
            );
        }
    }

    #[test]
    fn roam_double_brackets_parse() {
        assert_eq!(extract_links(&mask_noncontent("[[[[Ideas]]]]")), ["Ideas"]);
        assert_eq!(
            extract_links(&mask_noncontent("- [[[[Daily Notes]]]]")),
            ["Daily Notes"]
        );
    }
}
