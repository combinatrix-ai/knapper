//! One local link, as written, with the span it occupies in a file.
//!
//! This is the single occurrence-level reader. `move` uses it to rewrite links
//! a directory move invalidated, and `repair-links` uses it to point at the
//! ones that were already broken; both have to agree about what a link *is*,
//! down to the byte, or one of them would edit something the other never saw.
//!
//! It is deliberately not a second link parser. `parser` decides what a target
//! *means* -- which text is a reference at all, and what it normalises to --
//! and this module answers the separate question of *where* that reference
//! sits in the file. The two meet in `repair`, which normalises every span it
//! finds through `parser` rather than through a rule of its own.

use std::ops::Range;
use std::sync::LazyLock;

use percent_encoding::percent_decode_str;
use regex::Regex;

use crate::note::split_frontmatter;
use crate::parser::mask_noncontent;

// [[target#anchor|alias]], optionally embedded.
static WIKI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(!)?\[\[([^\[\]|#^]*?)((?:#|\^)[^\[\]|]*)?(\|[^\[\]]*)?\]\]").unwrap()
});

/// What a bare markdown destination may contain, shared by every pattern in
/// knapper that reads one.
///
/// Three files used to spell this out separately, and every one of them was
/// wrong in a different way: one stopped at whitespace and lost `<With
/// Space.md>`, one stopped at the first `)` and read `Note%20(draft).md` as
/// `Note%20(draft`. CommonMark allows parentheses in an unbracketed
/// destination as long as they balance, and Obsidian writes them, so the
/// nesting is matched here rather than re-derived per file.
///
/// Two levels deep, which covers `Note (draft).md` and `Paper (Smith
/// (2020)).md`. A regex cannot balance to arbitrary depth; deeper than that,
/// the pattern stops matching and the link goes unseen rather than being read
/// wrong -- write such a path as `<...>`, which has no depth limit.
pub(crate) const DESTINATION: &str = r"(?:[^()\s]|\((?:[^()\s]|\([^()\s]*\))*\))+";

// [label](target "title"), optionally embedded, target optionally in <>.
static MD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(!)?\[([^\[\]]*)\]\(\s*(?:<([^>]*)>|({DESTINATION}))((?:[ \t]+"[^"]*")?)[ \t]*\)"#
    ))
    .unwrap()
});

static EXTERNAL_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z][a-zA-Z0-9+.\-]*:").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Wiki,
    Markdown,
}

impl Kind {
    /// The name this link syntax goes by in JSON output.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Wiki => "wikilink",
            Kind::Markdown => "markdown",
        }
    }
}

/// One local link, as written, with the span it occupies in the file.
#[derive(Debug, Clone)]
pub struct RawLink {
    pub range: Range<usize>,
    pub kind: Kind,
    /// `![[..]]` or `![](..)`.
    pub embed: bool,
    /// The markdown label, verbatim.
    pub label: String,
    /// The path part, percent-decoded, with no anchor.
    pub path: String,
    /// `#heading` or `^block-id`, verbatim and still encoded.
    pub anchor: String,
    /// A wikilink's `|alias`, bar included.
    pub alias: String,
    /// A markdown link's ` "title"`, leading space included.
    pub title: String,
    /// The href was written as `<...>`.
    pub angle: bool,
}

/// Where the body starts, given the body slice `split_frontmatter` returned.
///
/// `body` is always a subslice of `content`, so its address is its offset.
fn body_offset(content: &str, body: &str) -> usize {
    (body.as_ptr() as usize).saturating_sub(content.as_ptr() as usize)
}

/// Every local link in a note, with byte spans into `content`.
///
/// Frontmatter is scanned verbatim -- a wikilink written as a property value
/// is a real link -- and the body is scanned through `mask_noncontent`, so
/// fenced code, inline code, comments, outliner macros and block references
/// hold no links here, exactly as they hold none in the graph.
pub fn scan_links(content: &str) -> Vec<RawLink> {
    let (_, body) = split_frontmatter(content);
    let start = body_offset(content, body);
    let masked = format!("{}{}", &content[..start], mask_noncontent(body));
    debug_assert_eq!(masked.len(), content.len());

    let mut links: Vec<RawLink> = Vec::new();

    for c in WIKI.captures_iter(&masked) {
        let whole = c.get(0).unwrap();
        let mut path = c
            .get(2)
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();
        let mut anchor = c
            .get(3)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        let mut alias = c
            .get(4)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        // Inside a Markdown table Obsidian escapes the alias separator as
        // `\|`. The backslash protects table syntax; it is not part of the
        // note path or anchor. Keep it with the alias so rewrites round-trip.
        if !alias.is_empty() {
            let escaped = if anchor.ends_with('\\') {
                anchor.pop();
                true
            } else if path.ends_with('\\') {
                path.pop();
                true
            } else {
                false
            };
            if escaped {
                alias.insert(0, '\\');
            }
        }
        // Do not turn an empty pair of brackets into a local anchor. A
        // leading `#`/`^` is intentionally allowed so anchor-only links can
        // be validated against the note that contains them.
        if path.is_empty() && anchor.is_empty() {
            continue;
        }
        links.push(RawLink {
            range: whole.range(),
            kind: Kind::Wiki,
            embed: c.get(1).is_some(),
            label: String::new(),
            path,
            anchor,
            alias,
            title: String::new(),
            angle: false,
        });
    }

    let taken: Vec<Range<usize>> = links.iter().map(|l| l.range.clone()).collect();
    for c in MD.captures_iter(&masked) {
        let whole = c.get(0).unwrap();
        if taken
            .iter()
            .any(|r| whole.start() < r.end && r.start < whole.end())
        {
            continue;
        }
        let angle = c.get(3).is_some();
        let href = c.get(3).or_else(|| c.get(4)).unwrap().as_str();
        if EXTERNAL_SCHEME.is_match(href) || href.starts_with("//") {
            continue;
        }
        // Obsidian writes a block reference as Note.md#^id, so the anchor
        // starts at the first '#' and everything before it is the path.
        let (path, anchor) = match href.find('#') {
            Some(index) => (&href[..index], &href[index..]),
            _ => (href, ""),
        };
        if path.is_empty() && anchor.is_empty() {
            continue;
        }
        links.push(RawLink {
            range: whole.range(),
            kind: Kind::Markdown,
            embed: c.get(1).is_some(),
            label: c[2].to_string(),
            path: percent_decode_str(path).decode_utf8_lossy().into_owned(),
            anchor: anchor.to_string(),
            alias: String::new(),
            title: c.get(5).map(|m| m.as_str().to_string()).unwrap_or_default(),
            angle,
        });
    }

    links.sort_by_key(|l| l.range.start);
    links
}

/// Percent-encode the path part of a bare markdown href.
///
/// `link.path` holds the target already decoded, so what arrives here is a
/// real filename and every character in it is a literal. Anything the
/// inline-link syntax reads as punctuation has to go back out encoded, or the
/// rewrite produces something that is no longer the link it replaced.
///
/// Four characters are unsafe in *either* href form. `>` closes an angle
/// destination and `<` may not appear inside one; `#` would be taken for the
/// anchor this rewriter is careful to keep separate; and a literal `%` would
/// come back as the start of an escape -- `Note%.md` is not even well-formed
/// percent-encoding, and `Guide#1` re-read as an anchor loses the rest of the
/// path. Encoding these is what makes the scan-decode / render-encode pair a
/// round trip.
///
/// `bare` adds the ones only an undelimited destination minds: an unbalanced
/// paren ends it, a `"` after it opens a title, and a space does both. Inside
/// `<...>` all three are safe, which is the reason an author writes it that
/// way, so the angle form keeps them and stays legible.
///
/// Encoding is decided by the path being written, not by whether the old one
/// happened to be encoded: a plain link can acquire a paren purely by being
/// moved into a directory that has one in its name.
fn percent_encode(path: &str, bare: bool) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        let escaped = match c {
            '%' => Some("%25"),
            '#' => Some("%23"),
            '<' => Some("%3C"),
            '>' => Some("%3E"),
            ' ' if bare => Some("%20"),
            '(' if bare => Some("%28"),
            ')' if bare => Some("%29"),
            '"' if bare => Some("%22"),
            _ => None,
        };
        match escaped {
            Some(escaped) => out.push_str(escaped),
            None => out.push(c),
        }
    }
    out
}

pub fn encode_href_path(path: &str) -> String {
    percent_encode(path, true)
}

pub fn encode_angle_path(path: &str) -> String {
    percent_encode(path, false)
}

/// The same link, written to point at `path` instead.
pub fn render(link: &RawLink, path: &str) -> String {
    let bang = if link.embed { "!" } else { "" };
    match link.kind {
        Kind::Wiki => format!("{bang}[[{path}{}{}]]", link.anchor, link.alias),
        Kind::Markdown => {
            let href = if link.angle {
                format!("<{}{}>", encode_angle_path(path), link.anchor)
            } else {
                format!("{}{}", encode_href_path(path), link.anchor)
            };
            format!("{bang}[{}]({href}{})", link.label, link.title)
        }
    }
}

/// Keep the target spelled the way it was: with its extension if it had one,
/// without if it did not. A vault where notes are linked as `[[Note]]` should
/// not end up with one `[[Archive/Note.md]]` in it.
pub fn strip_unwritten_extension(written: &str, target: &str) -> String {
    let extension = std::path::Path::new(target)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_string();
    if extension.is_empty() {
        return target.to_string();
    }
    let had = written
        .to_lowercase()
        .ends_with(&format!(".{}", extension.to_lowercase()));
    if had {
        target.to_string()
    } else {
        target[..target.len() - extension.len() - 1].to_string()
    }
}

/// The 1-based line an offset falls on.
pub fn line_of(content: &str, offset: usize) -> usize {
    content[..offset].matches('\n').count() + 1
}

/// The 1-based column an offset falls on, counted in characters rather than
/// bytes, so a line of Japanese reports the column a reader would count.
pub fn column_of(content: &str, offset: usize) -> usize {
    let before = &content[..offset];
    let start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    before[start..].chars().count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything the inline-link syntax reads as punctuation has to come
    /// back out encoded, or the rewrite is no longer the link it replaced.
    #[test]
    fn a_rewritten_href_encodes_what_would_otherwise_end_it() {
        for (path, expected) in [
            ("Archive/Guide/Note.md", "Archive/Guide/Note.md"),
            ("Archive/My Guide/Note.md", "Archive/My%20Guide/Note.md"),
            ("Archive/Guide(x)/Note.md", "Archive/Guide%28x%29/Note.md"),
            ("Archive/Guide#1/Note.md", "Archive/Guide%231/Note.md"),
            ("Archive/100%/Note.md", "Archive/100%25/Note.md"),
            (
                "Archive/say \"hi\"/Note.md",
                "Archive/say%20%22hi%22/Note.md",
            ),
            ("Archive/<x>/Note.md", "Archive/%3Cx%3E/Note.md"),
            // Non-ASCII is legal in a destination and needs no escaping.
            ("Archive/日本語/Note.md", "Archive/日本語/Note.md"),
        ] {
            assert_eq!(encode_href_path(path), expected, "path: {path}");
        }
    }

    /// The angle form exists so spaces and punctuation can stay readable, so
    /// it keeps them -- but not the four that no href form can carry.
    #[test]
    fn the_angle_form_encodes_only_what_it_must() {
        for (path, expected) in [
            ("Archive/My Guide/Note.md", "Archive/My Guide/Note.md"),
            ("Archive/Guide(x)/Note.md", "Archive/Guide(x)/Note.md"),
            ("Archive/say \"hi\"/Note.md", "Archive/say \"hi\"/Note.md"),
            // These four would be misread wherever they appear.
            ("Archive/Guide#1/Note.md", "Archive/Guide%231/Note.md"),
            ("Archive/Note%.md", "Archive/Note%25.md"),
            ("Archive/<x>/Note.md", "Archive/%3Cx%3E/Note.md"),
        ] {
            assert_eq!(encode_angle_path(path), expected, "path: {path}");
        }
    }

    /// The encoder is the inverse of the decode the scanner did, so a path
    /// that was written encoded comes back spelled the same way -- in both
    /// href forms, since the scanner decodes both.
    #[test]
    fn encoding_round_trips_what_the_scanner_decoded() {
        for written in [
            "Guide%28x%29/Note.md",
            "My%20Guide/My%20Note.md",
            "Guide%231/Note.md",
            "100%25/Note.md",
        ] {
            let decoded = percent_decode_str(written).decode_utf8_lossy().into_owned();
            assert_eq!(encode_href_path(&decoded), written, "written: {written}");
        }

        // The angle form leaves spaces and parens alone, so only the paths
        // that need no such escape round-trip character for character.
        for written in ["Guide%231/Note%25.md", "Guide(x)/Note.md", "a b/c.md"] {
            let decoded = percent_decode_str(written).decode_utf8_lossy().into_owned();
            assert_eq!(encode_angle_path(&decoded), written, "written: {written}");
        }
    }

    #[test]
    fn a_link_keeps_the_extension_it_was_written_with() {
        assert_eq!(
            strip_unwritten_extension("Docs/A.md", "New/A.md"),
            "New/A.md"
        );
        assert_eq!(strip_unwritten_extension("Docs/A", "New/A.md"), "New/A");
        assert_eq!(
            strip_unwritten_extension("pic.png", "New/pic.png"),
            "New/pic.png"
        );
    }

    #[test]
    fn every_link_form_is_found_with_its_span() {
        let content = "\
---
related: \"[[Docs/README]]\"
---

[[Docs/README]] and [[Docs/README|label]] and ![[Docs/pic.png]]
[text](Docs/README.md) and ![alt](Docs/pic.png)
`[[Docs/README]]` and

```
[[Docs/README]]
```
";
        let found = scan_links(content);
        let rendered: Vec<&str> = found.iter().map(|l| &content[l.range.clone()]).collect();
        assert_eq!(
            rendered,
            [
                "[[Docs/README]]",
                "[[Docs/README]]",
                "[[Docs/README|label]]",
                "![[Docs/pic.png]]",
                "[text](Docs/README.md)",
                "![alt](Docs/pic.png)",
            ]
        );
    }

    #[test]
    fn table_escaped_alias_separator_is_not_part_of_the_target() {
        let links = scan_links("| [[#Heading\\|label]] | [[Note\\|alias]] |\n");
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].path, "");
        assert_eq!(links[0].anchor, "#Heading");
        assert_eq!(links[0].alias, "\\|label");
        assert_eq!(render(&links[0], ""), "[[#Heading\\|label]]");
        assert_eq!(links[1].path, "Note");
        assert_eq!(links[1].alias, "\\|alias");
        assert_eq!(render(&links[1], "Other"), "[[Other\\|alias]]");
    }

    /// A position is reported the way a reader counts it: lines from one,
    /// columns in characters.
    #[test]
    fn positions_are_one_based_and_counted_in_characters() {
        let content = "first\n日本語 [[X]]\n";
        let at = content.find("[[X]]").unwrap();
        assert_eq!(line_of(content, at), 2);
        assert_eq!(column_of(content, at), 5);
        assert_eq!(line_of(content, 0), 1);
        assert_eq!(column_of(content, 0), 1);
    }
}
