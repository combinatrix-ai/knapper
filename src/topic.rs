//! Soft topic references: the `#tag` selector.
//!
//! knapper has two kinds of reference and they mean different things.
//!
//! `[[X]]` is a **hard note reference**. It names a note, it becomes an edge
//! in the link graph, and a missing target is a broken link worth reporting.
//!
//! `#X` is a **soft topic reference**. It labels a note with a subject. It
//! promises nothing about a note called `X` existing, so it is never broken,
//! never an edge, and never an orphan or a hub.
//!
//! What this module adds is that the weaker one is still navigable. A `#tag`
//! argument to `backlinks` or `context` resolves to a *virtual* subject: the
//! occurrences that carry the tag, and the notes those occurrences live in.
//! Nothing about it pretends a note exists.
//!
//! Resolution is explicit. Only a leading `#` asks for a tag, so no tag
//! becomes an ordinary graph node by accident and no default report changes
//! shape because a vault happens to use tags.

use std::collections::BTreeSet;

use anyhow::{bail, Result};
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::commands::context_window;
use crate::note::{frontmatter_tags, split_frontmatter};
use crate::parser::{extract_tags, mask_noncontent};
use crate::vault::{all_notes, is_org, relative_path, Config};

fn print_json(value: &Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

// ------------------------------------------------------------- selectors --

/// What a subject argument names.
///
/// `backlinks` and `context` both take one. A leading `#` is the only thing
/// that makes it a tag; everything else is a note, exactly as before.
pub enum Selector {
    Note(String),
    Tag(TagSelector),
}

impl Selector {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.strip_prefix('#') {
            Some(name) => TagSelector::new(name).map(Selector::Tag),
            None => Ok(Selector::Note(raw.to_string())),
        }
    }
}

/// A tag, as a subject to look up.
#[derive(Debug, Clone)]
pub struct TagSelector {
    tag: String,
}

impl TagSelector {
    pub fn new(name: &str) -> Result<Self> {
        let name = name.trim();
        if !is_taggable(name) {
            bail!(
                "Not a tag: {:?}\n\
                 A tag is written the way a note writes it: letters, digits, _ and -, \
                 with / for nesting and no spaces. It also needs a letter, so #2026/07 \
                 is a date rather than a tag.",
                format!("#{name}")
            );
        }
        Ok(Self {
            tag: name.to_string(),
        })
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// How the selector is written back to the user.
    pub fn selector(&self) -> String {
        format!("#{}", self.tag)
    }

    /// True when `tag` is this tag, or one nested under it.
    ///
    /// Nesting is the only widening: `#work` covers `#work/hiring`, because
    /// that is what writing a nested tag means. Matching is otherwise exact
    /// and case-sensitive, and no Unicode normalisation is applied -- the
    /// same rule `tags --find` follows, so one vault cannot have two answers
    /// to "which notes carry this tag".
    pub fn matches(&self, tag: &str) -> bool {
        match tag.strip_prefix(&self.tag) {
            Some("") => true,
            Some(rest) => rest.starts_with('/'),
            None => false,
        }
    }
}

/// True when `name` is a tag knapper's own parser would find.
///
/// Asking the parser rather than restating its character class is the point:
/// a name is taggable exactly when writing `#name` produces `name` back. That
/// keeps the selector, the extractor and the rewriter from ever disagreeing
/// about what a tag is -- including the Unicode ones, where restating the
/// rule by hand is how a CJK tag stops being valid.
pub fn is_taggable(name: &str) -> bool {
    !name.is_empty() && extract_tags(&format!(" #{name}")) == [name]
}

// ----------------------------------------------------------- occurrences --

/// How a note carries a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Carrier {
    /// Written as `#tag` in the note's prose.
    Inline,
    /// Declared as metadata: YAML `tags:`, or org `#+filetags:` and heading
    /// tags. The same topic, in the syntax the flavor uses for it.
    Declared,
}

impl Carrier {
    pub fn as_str(self) -> &'static str {
        match self {
            Carrier::Inline => "inline",
            Carrier::Declared => "declared",
        }
    }
}

/// One place a tag is carried. `line` is 1-based and file-relative, so it
/// points at the same line an editor would open.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Occurrence {
    pub source: String,
    pub line: usize,
    pub tag: String,
    pub carrier: Carrier,
    /// The source line itself, trimmed.
    pub text: String,
}

/// Where the body starts, as a 0-based line count. Frontmatter occupies the
/// lines before it, and a body line number that ignored them would point at
/// the wrong line of the file.
fn body_line_offset(content: &str, body: &str) -> usize {
    match content.len().checked_sub(body.len()) {
        Some(prefix) if content.is_char_boundary(prefix) => content[..prefix].matches('\n').count(),
        _ => 0,
    }
}

fn line_text(lines: &[&str], line: usize) -> String {
    lines
        .get(line.saturating_sub(1))
        .map(|l| l.trim().to_string())
        .unwrap_or_default()
}

/// The line a declared tag is written on, searched within `within` lines.
///
/// A YAML list puts each tag on its own line and an inline list puts them all
/// on one; org writes them on a heading or a `#+filetags:` line. Rather than
/// model each, look for the line that holds the text, and fall back to the
/// first line when a flavor writes it somewhere this cannot see.
fn declared_line(lines: &[&str], within: usize, tag: &str) -> usize {
    lines[..within.min(lines.len())]
        .iter()
        .position(|line| line.contains(tag))
        .map(|index| index + 1)
        .unwrap_or(1)
}

fn occurrences_in(
    config: &Config,
    path: &std::path::Path,
    selector: &TagSelector,
) -> Vec<Occurrence> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let source = relative_path(&config.vault_path, path);
    let lines: Vec<&str> = content.lines().collect();
    let mut found = Vec::new();

    let declared = |tag: String, within: usize, found: &mut Vec<Occurrence>| {
        let line = declared_line(&lines, within, &tag);
        found.push(Occurrence {
            source: source.clone(),
            line,
            tag,
            carrier: Carrier::Declared,
            text: line_text(&lines, line),
        });
    };

    // org has no `#tag` prose syntax at all: its tags are `:like:this:` on a
    // heading, or a `#+filetags:` line. Both are declarations, and scanning
    // org prose for `#tag` would only invent occurrences.
    if is_org(path) {
        for tag in crate::org::parse_org(&content).tags {
            if selector.matches(&tag) {
                declared(tag, lines.len(), &mut found);
            }
        }
        return found;
    }

    let (metadata, body) = split_frontmatter(&content);
    let offset = body_line_offset(&content, body);
    for tag in frontmatter_tags(&metadata) {
        if selector.matches(&tag) {
            declared(tag, offset, &mut found);
        }
    }

    // Masking keeps byte offsets and line numbers, so a tag inside a fence,
    // an inline span or a %%comment%% is not an occurrence, and everything
    // else still reports the line it is really on.
    let masked = mask_noncontent(body);
    for (index, line) in masked.lines().enumerate() {
        for tag in extract_tags(line) {
            if !selector.matches(&tag) {
                continue;
            }
            let line = offset + index + 1;
            found.push(Occurrence {
                source: source.clone(),
                line,
                tag,
                carrier: Carrier::Inline,
                text: line_text(&lines, line),
            });
        }
    }

    found
}

/// Every occurrence of the selected tag in the vault.
///
/// Ordered by source path, then line, then tag, so a caller reading the JSON
/// gets the same list every run. Excluded notes are not scanned, because they
/// are not the user's notes anywhere else either.
pub fn occurrences(config: &Config, selector: &TagSelector) -> Vec<Occurrence> {
    let mut found: Vec<Occurrence> = all_notes(config)
        .par_iter()
        .flat_map(|path| occurrences_in(config, path, selector))
        .collect();
    found.sort();
    // One occurrence per note, line and tag: a line that writes the same tag
    // twice says one thing about that line.
    found.dedup_by(|a, b| (&a.source, a.line, &a.tag) == (&b.source, b.line, &b.tag));
    found
}

/// The notes carrying the tag, in path order.
pub fn sources(found: &[Occurrence]) -> Vec<String> {
    let unique: BTreeSet<&String> = found.iter().map(|o| &o.source).collect();
    unique.into_iter().cloned().collect()
}

// -------------------------------------------------------------- commands --

/// `backlinks '#tag'`: the notes and lines that carry a tag.
///
/// The same three formats as note backlinks, with the same `-A`/`-B`
/// behaviour. Each entry names the tag it actually matched, which is what
/// tells an exact hit from a nested one.
pub fn backlinks(
    config: &Config,
    selector: &TagSelector,
    format: &str,
    before: usize,
    after: usize,
) -> Result<()> {
    let found = occurrences(config, selector);

    match format {
        "json" => {
            let items: Vec<Value> = found
                .iter()
                .map(|occurrence| {
                    let mut entry = json!({
                        "source": occurrence.source,
                        "line": occurrence.line,
                        "tag": occurrence.tag,
                        "where": occurrence.carrier.as_str(),
                    });
                    if before > 0 || after > 0 {
                        if let Ok(content) =
                            std::fs::read_to_string(config.vault_path.join(&occurrence.source))
                        {
                            let lines: Vec<&str> = content.lines().collect();
                            if let Some(window) =
                                context_window(&lines, occurrence.line, before, after)
                            {
                                entry["context"] = json!(window);
                            }
                        }
                    }
                    entry
                })
                .collect();
            print_json(&Value::Array(items));
        }
        "paths" => sources(&found).iter().for_each(|s| println!("{s}")),
        _ => {
            for occurrence in &found {
                println!(
                    "\n{} (line {}) #{}",
                    occurrence.source, occurrence.line, occurrence.tag
                );
                if before > 0 || after > 0 {
                    if let Ok(content) =
                        std::fs::read_to_string(config.vault_path.join(&occurrence.source))
                    {
                        let lines: Vec<&str> = content.lines().collect();
                        if let Some(window) = context_window(&lines, occurrence.line, before, after)
                        {
                            window.split('\n').for_each(|l| println!("  {l}"));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// `context '#tag'`: the whole subject, as one document.
///
/// A tag has no note, so this has no `path`, no `content` and no headings.
/// What it has is what a tag actually is: the notes that carry it, where they
/// carry it, the tags nested under it, and the tasks that mention it.
/// `kind: "tag"` is there so a caller never has to guess which shape it got.
pub fn context(
    config: &Config,
    selector: &TagSelector,
    format: &str,
    options: &crate::notes_cmd::ContextOptions,
) -> Result<()> {
    let found = occurrences(config, selector);
    let notes = sources(&found);

    let nested: Vec<String> = {
        let unique: BTreeSet<&String> = found
            .iter()
            .map(|o| &o.tag)
            .filter(|tag| *tag != selector.tag())
            .collect();
        unique.into_iter().cloned().collect()
    };

    let text_of = |occurrence: &Occurrence| match options.max_content {
        Some(limit) if occurrence.text.chars().count() > limit => {
            let head: String = occurrence.text.chars().take(limit).collect();
            format!("{head} ... (truncated)")
        }
        _ => occurrence.text.clone(),
    };

    let tasks: Vec<crate::tasks::Task> = if options.no_tasks {
        Vec::new()
    } else {
        let filters = crate::tasks::Filters {
            include_done: true,
            exclude: &[],
            status: &[],
            ..Default::default()
        };
        crate::tasks::find_tasks(config, &filters)
            .unwrap_or_default()
            .into_iter()
            .filter(|task| task.tags.iter().any(|tag| selector.matches(tag)))
            .collect()
    };

    if format == "json" {
        let mut out = serde_json::Map::new();
        out.insert("kind".into(), json!("tag"));
        out.insert("tag".into(), json!(selector.tag()));
        out.insert("selector".into(), json!(selector.selector()));
        out.insert("notes".into(), json!(notes));
        if !nested.is_empty() {
            out.insert("nested_tags".into(), json!(nested));
        }
        out.insert(
            "occurrences".into(),
            json!(found
                .iter()
                .map(|occurrence| {
                    let mut entry = json!({
                        "source": occurrence.source,
                        "line": occurrence.line,
                        "tag": occurrence.tag,
                        "where": occurrence.carrier.as_str(),
                    });
                    if !options.no_content {
                        entry["text"] = json!(text_of(occurrence));
                    }
                    entry
                })
                .collect::<Vec<_>>()),
        );
        if !tasks.is_empty() {
            out.insert(
                "tasks".into(),
                json!(tasks
                    .iter()
                    .map(
                        |t| json!({"file": t.file, "line": t.line, "text": t.text, "done": t.done})
                    )
                    .collect::<Vec<_>>()),
            );
        }
        out.insert(
            "stats".into(),
            json!({"notes": notes.len(), "occurrences": found.len()}),
        );
        print_json(&Value::Object(out));
        return Ok(());
    }

    println!("# {}\n", selector.selector());
    println!("A tag, not a note: nothing needs to exist for it to resolve.\n");

    println!("## Notes ({})", notes.len());
    notes.iter().for_each(|note| println!("  {note}"));

    println!("\n## Occurrences ({})", found.len());
    for occurrence in &found {
        print!(
            "  {}:{} #{}",
            occurrence.source, occurrence.line, occurrence.tag
        );
        match options.no_content {
            true => println!(),
            false => println!("  {}", text_of(occurrence)),
        }
    }

    if !nested.is_empty() {
        println!("\n## Nested tags");
        nested.iter().for_each(|tag| println!("  #{tag}"));
    }

    if !tasks.is_empty() {
        println!("\n## Tasks ({})", tasks.len());
        for task in &tasks {
            let mark = if task.done { "x" } else { " " };
            println!("  {}:{} [{mark}] {}", task.file, task.line, task.text);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selector(raw: &str) -> TagSelector {
        TagSelector::new(raw).unwrap()
    }

    /// A tag argument is the one the vault already writes, so the CJK tags a
    /// Japanese daily note is full of have to survive the trip.
    #[test]
    fn a_selector_accepts_every_tag_the_parser_finds() {
        for name in [
            "work",
            "COO採用",
            "日本語タグ",
            "中文标签",
            "has_underscore",
            "with-dash",
            "parent/child",
            "parent/child/grand",
            "v2beta",
        ] {
            assert!(is_taggable(name), "should be a tag: {name}");
            assert_eq!(selector(name).tag(), name);
        }
    }

    /// The refusals are the ones the parser would make anyway. Accepting a
    /// name the extractor cannot find would produce a selector that matches
    /// nothing and says nothing about why.
    #[test]
    fn a_selector_refuses_what_the_parser_would_not_find() {
        for name in [
            "",
            "two words",
            "trailing/",
            "123",
            "8/18",
            "2026/07",
            "has.dot",
            "with#hash",
            "[[wikilink]]",
            "a\nb",
        ] {
            assert!(!is_taggable(name), "should not be a tag: {name}");
            assert!(TagSelector::new(name).is_err(), "should refuse: {name}");
        }
    }

    #[test]
    fn only_a_leading_hash_selects_a_tag() {
        assert!(matches!(
            Selector::parse("Notes/Lit Review.md").unwrap(),
            Selector::Note(_)
        ));
        assert!(matches!(
            Selector::parse("#COO採用").unwrap(),
            Selector::Tag(_)
        ));
        // A note argument is never validated as a tag, so a filename that
        // could not be one still reaches the note path.
        assert!(matches!(
            Selector::parse("2026/07.md").unwrap(),
            Selector::Note(_)
        ));
    }

    /// Nesting is the one widening. `#work` is what a note means when it
    /// writes `#work/hiring`, so a lookup for the parent finds the child --
    /// and every entry says which tag it actually matched.
    #[test]
    fn a_selector_matches_itself_and_its_nested_children() {
        let work = selector("work");
        assert!(work.matches("work"));
        assert!(work.matches("work/hiring"));
        assert!(work.matches("work/hiring/coo"));

        assert!(!work.matches("workshop"));
        assert!(!work.matches("homework"));
        assert!(!work.matches("home/work"));
        // Case is not folded: `tags` and `tags --find` do not fold it either.
        assert!(!work.matches("Work"));
    }

    #[test]
    fn a_nested_selector_does_not_reach_back_up() {
        let child = selector("work/hiring");
        assert!(child.matches("work/hiring"));
        assert!(child.matches("work/hiring/coo"));
        assert!(!child.matches("work"));
    }

    #[test]
    fn a_cjk_selector_matches_only_the_whole_tag() {
        let hiring = selector("COO採用");
        assert!(hiring.matches("COO採用"));
        assert!(hiring.matches("COO採用/面接"));
        assert!(!hiring.matches("COO採用計画"));
        assert!(!hiring.matches("採用"));
    }

    /// A body line number has to be a file line number, or every editor jump
    /// lands a few lines short of the tag.
    #[test]
    fn a_body_line_offset_counts_the_frontmatter() {
        let content = "---\ntags: [a]\n---\n\nbody #a\n";
        let (_, body) = split_frontmatter(content);
        assert_eq!(body_line_offset(content, body), 4);

        let plain = "no frontmatter #a\n";
        let (_, body) = split_frontmatter(plain);
        assert_eq!(body_line_offset(plain, body), 0);
    }
}
