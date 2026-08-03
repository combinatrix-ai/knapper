//! External references: `knapper://<provider>/<locator>` links in notes.
//!
//! A note holds an ordinary markdown link whose destination names a provider
//! and an opaque locator:
//!
//! ```markdown
//! [日本橋小舟町の住所](knapper://personal/address.nihonbashi_kobunacho)
//! ```
//!
//! The provider is a name the user chose, not a tool: what runs for it lives
//! in the local provider config that `providers.rs` owns, never in the vault.
//! This file owns the grammar and finds references in notes. It resolves
//! nothing and never sees a value.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::LazyLock;

use anyhow::{anyhow, Result};
use regex::Regex;
use serde::Serialize;

use crate::parser::mask_noncontent;
use crate::vault::{all_notes, relative_path, resolve_path, Config};

pub const SCHEME: &str = "knapper://";

/// A locator is bounded so a reference stays a reference: something short
/// enough to read in a note, rather than a payload smuggled through a link.
pub const MAX_LOCATOR: usize = 128;

// The whole URI, anchored. The provider is lowercase because it is a name the
// user types, and a case-folding provider name is a name with two spellings.
// The locator's alphabet is deliberately small and ASCII: knapper hands it to
// somebody else's command, so anything it cannot spell cannot be smuggled.
// The anchors are \A and \z rather than ^ and $ so that nothing -- a trailing
// newline included -- can sit outside the match.
static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\Aknapper://([a-z0-9][a-z0-9_-]*)/([A-Za-z0-9][A-Za-z0-9._/-]{0,127})\z").unwrap()
});

static PROVIDER_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[a-z0-9][a-z0-9_-]*\z").unwrap());

// A markdown inline link whose destination is a knapper URI, in both the
// plain and the angle-wrapped form. A bare URI in prose and a markdown
// autolink are not references: a reference is something the note links to,
// and requiring the link form keeps a URI that is merely being discussed from
// becoming one.
static MARKDOWN_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\[([^\]]*)\]\(\s*(?:<(knapper://[^<>\s]*)>|(knapper://[^()<>\s]*))\s*(?:"[^"]*")?\s*\)"#,
    )
    .unwrap()
});

/// One reference, as written. There is no value here and never will be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reference {
    pub path: String,
    pub line: usize,
    /// Column of the URI itself, not of the link that carries it.
    pub column: usize,
    pub uri: String,
    pub provider: String,
    pub locator: String,
    pub label: String,
}

/// Split a reference URI into its provider and locator.
///
/// The locator is opaque: it is not decoded, unescaped or normalised, because
/// the provider defines what it means. Everything this checks is about what
/// knapper is willing to pass on at all.
pub fn parse(uri: &str) -> Result<(String, String)> {
    let captures = REFERENCE.captures(uri).ok_or_else(|| {
        anyhow!("Not a knapper reference: {uri:?} (expected {SCHEME}<provider>/<locator>)")
    })?;
    let provider = captures[1].to_string();
    let locator = captures[2].to_string();

    // The alphabet already excludes "%", "?", "#", whitespace and a leading
    // hyphen. What it cannot express is that every path segment must name
    // something: "a//b" and "a/../b" match it and mean nothing good.
    for segment in locator.split('/') {
        if segment.is_empty() {
            return Err(anyhow!("Empty path segment in locator: {locator:?}"));
        }
        if segment == "." || segment == ".." {
            return Err(anyhow!("Relative path segment in locator: {locator:?}"));
        }
    }

    Ok((provider, locator))
}

pub fn validate_provider(name: &str) -> Result<()> {
    if PROVIDER_NAME.is_match(name) {
        return Ok(());
    }
    Err(anyhow!(
        "Invalid provider name: {name:?} (lowercase letters, digits, '_' and '-', starting with a letter or digit)"
    ))
}

/// Every reference in one note, in reading order.
pub fn parse_refs(path: &str, content: &str) -> Vec<Reference> {
    let masked = mask_noncontent(content);
    let mut refs = Vec::new();

    for (line_index, line) in masked.lines().enumerate() {
        let bytes = line.as_bytes();
        for captures in MARKDOWN_REFERENCE.captures_iter(line) {
            let whole = captures.get(0).expect("the regex has a whole match");
            // No lookbehind in this engine, so check for the image "!"
            // directly. An embed is not an inline link.
            if whole.start() > 0 && bytes[whole.start() - 1] == b'!' {
                continue;
            }

            let destination = captures
                .get(2)
                .or_else(|| captures.get(3))
                .expect("one destination branch matched");
            let Ok((provider, locator)) = parse(destination.as_str()) else {
                continue;
            };

            refs.push(Reference {
                path: path.to_string(),
                line: line_index + 1,
                column: line[..destination.start()].chars().count() + 1,
                uri: destination.as_str().to_string(),
                provider,
                locator,
                label: captures[1].trim().to_string(),
            });
        }
    }
    refs
}

fn collect(config: &Config, file: Option<&str>) -> Result<Vec<Reference>> {
    let paths: Vec<PathBuf> = match file {
        Some(file) => {
            let path = resolve_path(&config.vault_path, file);
            if !path.exists() {
                return Err(anyhow!("File not found: {}", path.display()));
            }
            vec![path]
        }
        None => all_notes(config),
    };

    let mut refs = Vec::new();
    for path in paths {
        let content = std::fs::read_to_string(&path)?;
        let relative = relative_path(&config.vault_path, &path);
        refs.extend(parse_refs(&relative, &content));
    }
    refs.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.column.cmp(&b.column))
    });
    Ok(refs)
}

fn print(refs: &[Reference], format: &str) {
    match format {
        "json" => println!("{}", serde_json::to_string_pretty(refs).unwrap()),
        "paths" => {
            let paths: BTreeSet<&str> = refs.iter().map(|r| r.path.as_str()).collect();
            paths.iter().for_each(|path| println!("{path}"));
        }
        _ => refs.iter().for_each(|r| {
            if r.label.is_empty() {
                println!("{}:{}:{}  {}", r.path, r.line, r.column, r.uri);
            } else {
                println!("{}:{}:{}  {}  {}", r.path, r.line, r.column, r.uri, r.label);
            }
        }),
    }
}

pub fn refs(
    config: &Config,
    file: Option<&str>,
    provider: Option<&str>,
    format: &str,
) -> Result<()> {
    if let Some(provider) = provider {
        validate_provider(provider)?;
    }
    let found: Vec<Reference> = collect(config, file)?
        .into_iter()
        .filter(|reference| match provider {
            Some(name) => reference.provider == name,
            None => true,
        })
        .collect();
    print(&found, format);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_reference_splits_into_provider_and_locator() {
        for (uri, provider, locator) in [
            (
                "knapper://personal/address.nihonbashi_kobunacho",
                "personal",
                "address.nihonbashi_kobunacho",
            ),
            (
                "knapper://work/tokens/ci/deploy",
                "work",
                "tokens/ci/deploy",
            ),
            ("knapper://p1/A", "p1", "A"),
            ("knapper://a-b_c/x-y_z.0", "a-b_c", "x-y_z.0"),
        ] {
            assert_eq!(
                parse(uri).unwrap(),
                (provider.to_string(), locator.to_string()),
                "input: {uri}"
            );
        }
    }

    /// The locator reaches somebody else's command, so everything knapper is
    /// unsure about is a rejection rather than a guess.
    #[test]
    fn anything_outside_the_grammar_is_rejected() {
        for uri in [
            // Wrong scheme, or no scheme at all.
            "https://example.com/x",
            "knapper:/personal/x",
            "knapper:personal/x",
            "personal/x",
            "",
            // Missing halves.
            "knapper://personal",
            "knapper://personal/",
            "knapper:///x",
            "knapper://",
            // Provider shape.
            "knapper://Personal/x",
            "knapper://-personal/x",
            "knapper://_personal/x",
            "knapper://per.sonal/x",
            "knapper://per sonal/x",
            // Locator shape.
            "knapper://personal/-leading",
            "knapper://personal/.leading",
            "knapper://personal/_leading",
            "knapper://personal/has space",
            "knapper://personal/日本語",
            "knapper://personal/a:b",
            "knapper://personal/a\\b",
            // Percent encoding, query and fragment are not part of it.
            "knapper://personal/a%2Fb",
            "knapper://personal/a%20b",
            "knapper://personal/a?q=1",
            "knapper://personal/a#frag",
            // Path segments that name nothing.
            "knapper://personal/a//b",
            "knapper://personal/a/",
            "knapper://personal/a/./b",
            "knapper://personal/a/../b",
            "knapper://personal/a/..",
            // Trailing junk after an otherwise valid reference.
            "knapper://personal/x ",
            " knapper://personal/x",
            "knapper://personal/x\n",
        ] {
            assert!(parse(uri).is_err(), "should be rejected: {uri:?}");
        }
    }

    #[test]
    fn a_locator_is_bounded() {
        let longest = "a".repeat(MAX_LOCATOR);
        assert!(parse(&format!("{SCHEME}personal/{longest}")).is_ok());
        assert!(parse(&format!("{SCHEME}personal/{longest}a")).is_err());
    }

    /// A dotted locator is not a path to normalise: "a..b" is one segment and
    /// means whatever the provider says it means.
    #[test]
    fn dots_inside_a_segment_are_not_traversal() {
        assert!(parse("knapper://personal/a..b").is_ok());
        assert!(parse("knapper://personal/v1.2.3").is_ok());
    }

    #[test]
    fn provider_names_follow_the_same_shape_as_in_a_uri() {
        for name in ["personal", "work", "a", "p1", "a-b_c"] {
            assert!(validate_provider(name).is_ok(), "should accept: {name}");
        }
        for name in ["", "Personal", "-p", "_p", "a.b", "a b", "a/b"] {
            assert!(validate_provider(name).is_err(), "should reject: {name:?}");
        }
    }

    // ---------------------------------------------------------- discovery --

    #[test]
    fn both_link_forms_are_found_with_their_position() {
        let refs = parse_refs(
            "Note.md",
            "住所: [日本橋小舟町の住所](knapper://personal/address.nihonbashi_kobunacho)\n\
             Token: [CI](<knapper://work/tokens/ci.deploy>)\n",
        );
        assert_eq!(refs.len(), 2);

        assert_eq!(refs[0].line, 1);
        assert_eq!(refs[0].column, 17);
        assert_eq!(refs[0].provider, "personal");
        assert_eq!(refs[0].locator, "address.nihonbashi_kobunacho");
        assert_eq!(refs[0].label, "日本橋小舟町の住所");

        assert_eq!(refs[1].line, 2);
        assert_eq!(refs[1].uri, "knapper://work/tokens/ci.deploy");
        assert_eq!(refs[1].label, "CI");
    }

    /// The same masking that keeps a code sample out of the link graph keeps
    /// it out of here: an example in documentation is not a reference.
    #[test]
    fn code_and_comments_hold_no_references() {
        let refs = parse_refs(
            "Note.md",
            "`[x](knapper://personal/inline)`\n\
             %% [x](knapper://personal/comment) %%\n\
             ```\n\
             [x](knapper://personal/fenced)\n\
             ```\n\
             [x](knapper://personal/real)\n",
        );
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].locator, "real");
        assert_eq!(refs[0].line, 6);
    }

    /// v1 recognises links only. A URI sitting in prose is being talked
    /// about, not linked to.
    #[test]
    fn bare_uris_and_autolinks_are_not_references() {
        for content in [
            "see knapper://personal/bare for details",
            "<knapper://personal/autolink>",
            "[label]: knapper://personal/refdef",
            "![embed](knapper://personal/image)",
        ] {
            assert!(
                parse_refs("Note.md", content).is_empty(),
                "should not be a reference: {content}"
            );
        }
    }

    #[test]
    fn a_malformed_destination_is_not_a_reference() {
        let content = "[a](knapper://Personal/upper) [b](knapper://personal/a//b) \
                       [c](knapper://personal/../etc) [d](knapper://personal/a%2Fb) \
                       [e](knapper://personal/ok)";
        let refs = parse_refs("Note.md", content);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].locator, "ok");
    }

    #[test]
    fn a_link_title_does_not_end_up_in_the_uri() {
        let refs = parse_refs("Note.md", "[a](knapper://personal/x \"a title\")");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].uri, "knapper://personal/x");
    }

    #[test]
    fn several_references_share_a_line() {
        let refs = parse_refs(
            "Note.md",
            "[a](knapper://personal/a) and [b](knapper://work/b)",
        );
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].column, 5);
        assert_eq!(refs[1].column, 35);
        assert_eq!(refs[1].provider, "work");
    }
}
