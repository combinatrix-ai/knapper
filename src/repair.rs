//! Planning repairs for links that are already broken.
//!
//! A vault that has outlived two or three tools carries links nobody wrote
//! wrong: a folder was reorganised outside knapper, an exporter wrote a path
//! that has since moved, an encoding survived a round trip it should not have.
//! `rename` and `move` keep links intact through a refactor knapper performs;
//! this is the other half -- the links that broke while knapper was not
//! looking.
//!
//! What it will not do is guess. A repair is proposed only when the filesystem
//! alone settles it: exactly one file, reached by an exact structural
//! transformation of the target as written. `[[Roam]]` next to a note called
//! `RoamResearch` is a rename somebody performed in their head, and no amount
//! of string distance turns that into evidence. Those stay unresolved, and so
//! do missing dates and citation labels, which name no note at all.
//!
//! V1 is strictly read-only: it emits a plan and writes nothing. Every
//! proposed edit carries the byte span, the text before and the text after, so
//! an apply can be built on this plan rather than on a second scan.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{bail, Result};
use percent_encoding::percent_decode_str;
use rayon::prelude::*;
use regex::Regex;
use serde_json::{json, Value};

use crate::graph::{
    build_resolver, markdown_link_occurrences, IgnoredLinks, LinkResolver, MISSING_BLOCK,
    MISSING_HEADING,
};
use crate::links::{column_of, line_of, render, strip_unwritten_extension, Kind, RawLink};
use crate::org;
use crate::parser::{normalize_markdown_path, normalize_wikilink_target, NOTE_SUFFIXES};
use crate::vault::{all_files, all_notes, is_org, relative_path, Config};

/// What a caller may rely on in the JSON. These strings are the contract.
pub const SAFE: &str = "safe";
pub const AMBIGUOUS: &str = "ambiguous";
pub const UNRESOLVED: &str = "unresolved";

/// Why the link is broken -- a property of the target, not of the repair.
const MISSING_NOTE: &str = "missing-note";
const MISSING_PATH: &str = "missing-path";
const MISSING_DATE: &str = "missing-date";
const NUMERIC_LABEL: &str = "numeric-label";
const ORG_LINK: &str = "org-link";

/// What makes a candidate a candidate. Only the first two are ever safe.
///
/// One basis is deliberately absent: replacing a *wrong* extension. Note
/// extensions need no repair -- the resolver already answers `[[notes/Foo.md]]`
/// and `[[notes/Foo]]` alike -- so the only thing such a rule could add is
/// dropping an extension that is not a note's, and `old/Foo.txt` becoming
/// `notes/Foo.md` is a claim about what the author meant by `.txt`. It would
/// also re-enter the resolver's basename fallback, which is the judgement this
/// command exists to avoid. If a vault turns out to need it, it wants its own
/// basis and its own cases, not a widening of these.
const UNIQUE_PATH_SUFFIX: &str = "unique-path-suffix";
const PERCENT_DECODING: &str = "percent-decoding";
const PATH_SUFFIX: &str = "path-suffix";
const BASENAME: &str = "basename";

// A target that names a day rather than a note: 2026-07-04, 2026/07/04,
// 20260704, 2026-07. Daily notes that were never created are the largest
// single source of broken links in an imported vault, and inventing one is
// not a repair.
static DATE_LIKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\d{4}([-/.]\d{1,2}){1,2}$|^\d{8}$|^\d{1,2}[-/]\d{1,2}$").unwrap()
});

// A citation label an exporter left behind: 12, [3], (7).
static NUMERIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\[(]?\d+[\])]?$").unwrap());

// ------------------------------------------------------------- the records --

/// A file this occurrence might have meant, and the evidence for it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Candidate {
    pub path: String,
    pub basis: &'static str,
    /// The transformed target that produced the match, so a reader can see
    /// exactly what was compared.
    pub matched: String,
}

/// Everything an apply would need, without rescanning.
#[derive(Debug, Clone)]
pub struct Edit {
    pub byte_start: usize,
    pub byte_end: usize,
    pub before: String,
    pub after: String,
    pub target_before: String,
    pub target_after: String,
    pub resolves_to: String,
}

/// One broken link, where it sits, and what could be done about it.
#[derive(Debug, Clone)]
pub struct Occurrence {
    pub source: String,
    pub line: usize,
    /// 1-based, in characters. Absent for org, whose masking does not
    /// preserve byte offsets.
    pub column: Option<usize>,
    pub syntax: &'static str,
    /// The link exactly as written, brackets included.
    pub raw: String,
    /// The target exactly as written, before normalisation.
    pub raw_target: String,
    /// The target the resolver was asked about.
    pub target: String,
    pub reason: &'static str,
    pub status: &'static str,
    pub candidates: Vec<Candidate>,
    pub edit: Option<Edit>,
    /// Why an occurrence with candidates is not safe, in one sentence.
    pub note: Option<String>,
}

impl Occurrence {
    /// The sort key. Two links can share a line, so the span disambiguates.
    fn key(&self) -> (&str, usize, usize, &str) {
        (&self.source, self.line, self.column.unwrap_or(0), &self.raw)
    }
}

// ------------------------------------------------------------ destinations --

/// Every file a link could land on, indexed by path components.
///
/// Both spellings of a note are indexed -- `notes/Foo.md` and `notes/Foo` --
/// because a link may be written either way and the resolver accepts both.
/// Non-note leaves are indexed under their full name only, which is the same
/// rule the resolver follows: an attachment resolves by path, never by stem.
struct Destinations {
    entries: Vec<(Vec<String>, String)>,
}

/// The components a path is compared by: `.` and a stale `..` are dropped
/// rather than followed, and case is folded because resolution folds it.
fn parts(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .collect()
}

fn components(path: &str) -> Vec<String> {
    parts(path).into_iter().map(str::to_lowercase).collect()
}

impl Destinations {
    fn build(files: &BTreeSet<String>) -> Self {
        let mut entries = Vec::with_capacity(files.len() * 2);
        for file in files {
            entries.push((components(file), file.clone()));
            if let Some((stem, ext)) = file.rsplit_once('.') {
                if NOTE_SUFFIXES.contains(&ext.to_ascii_lowercase().as_str())
                    || ext.eq_ignore_ascii_case("org")
                {
                    entries.push((components(stem), file.clone()));
                }
            }
        }
        Self { entries }
    }

    fn ending_with(&self, suffix: &[String]) -> BTreeSet<String> {
        self.entries
            .iter()
            .filter(|(parts, _)| parts.len() >= suffix.len() && parts.ends_with(suffix))
            .map(|(_, path)| path.clone())
            .collect()
    }

    /// The longest path suffix of `target` that any file ends with, and the
    /// files that end with it. The suffix comes back spelled as the target
    /// spelled it, since that is the text a reader is being asked to check.
    ///
    /// Leading `..` and `.` are dropped rather than followed: a stale
    /// traversal is exactly the prefix this is looking past, and no candidate
    /// can come from outside the vault because every entry here is a file the
    /// vault walk found.
    fn longest_suffix(&self, target: &str) -> Option<(usize, String, BTreeSet<String>)> {
        let written = parts(target);
        let folded = components(target);
        for take in (1..=folded.len()).rev() {
            let hits = self.ending_with(&folded[folded.len() - take..]);
            if !hits.is_empty() {
                return Some((take, written[written.len() - take..].join("/"), hits));
            }
        }
        None
    }
}

// ------------------------------------------------------------- classifying --

fn reason_for(target: &str) -> &'static str {
    if DATE_LIKE.is_match(target) {
        MISSING_DATE
    } else if NUMERIC.is_match(target) {
        NUMERIC_LABEL
    } else if target.contains('/') {
        MISSING_PATH
    } else {
        MISSING_NOTE
    }
}

/// A path whose every directory component is a real directory inside the real
/// vault.
///
/// Candidates already come from a walk that does not descend symlinked
/// directories, so this cannot currently fail; it is here because a repair
/// writes a path into a note, and "the destination is inside the vault" is the
/// one property that must not depend on how the candidate list was built.
fn inside_real_vault(vault: Option<&PathBuf>, relative: &str) -> bool {
    let Some(vault) = vault else { return false };
    let mut current = vault.clone();
    let parts: Vec<&str> = relative.split('/').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        current = current.join(part);
        match current.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => return false,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    current
        .canonicalize()
        .map(|real| real.starts_with(vault))
        .unwrap_or(false)
}

/// The candidates for one broken target, and whether the evidence is the kind
/// that can ever be called safe.
fn candidates_for(
    resolver: &LinkResolver,
    destinations: &Destinations,
    source: &str,
    target: &str,
) -> (Vec<Candidate>, bool) {
    // An encoding that survived a round trip is not a guess: decoding it
    // yields the link the author wrote.
    //
    // It is asked `resolve_as_path` rather than `resolve`, because decoding
    // must not be a doorway to the fallbacks this command refuses. `resolve`
    // would answer a decoded `old%2FReport` with any note whose stem is
    // Report, and a decoded alias with the note declaring it -- a basename or
    // a semantic match wearing an encoding fix as a disguise. Only a
    // destination the decoded target names *as a path* is structural, and
    // anything else falls through to the suffix search below, where a
    // basename-only hit is a suggestion.
    let decoded = percent_decode_str(target).decode_utf8_lossy().into_owned();
    if decoded != target {
        if let Some(hit) = resolver.resolve_as_path(source, &decoded) {
            return (
                vec![Candidate {
                    path: hit,
                    basis: PERCENT_DECODING,
                    matched: decoded,
                }],
                true,
            );
        }
    }

    // A stale prefix: the tail of the path is still exactly right, and only
    // the directories in front of it moved.
    let found = destinations
        .longest_suffix(target)
        .or_else(|| destinations.longest_suffix(&decoded));
    let Some((take, matched, hits)) = found else {
        return (Vec::new(), false);
    };

    // One component is a basename, and a basename is a resemblance. Two or
    // more is a path: the directory it sits in has to agree as well, which is
    // structure rather than similarity.
    let basis = match (take, hits.len()) {
        (1, _) => BASENAME,
        (_, 1) => UNIQUE_PATH_SUFFIX,
        _ => PATH_SUFFIX,
    };
    let safe_eligible = basis == UNIQUE_PATH_SUFFIX;

    (
        hits.into_iter()
            .map(|path| Candidate {
                path,
                basis,
                matched: matched.clone(),
            })
            .collect(),
        safe_eligible,
    )
}

/// The edit that repairs this occurrence, if the rewritten link really does
/// resolve to the candidate.
///
/// The check is the point. Everything up to here is evidence about which file
/// was meant; this asks the resolver whether the text knapper would write
/// actually lands there, so a proposal can never be published on the strength
/// of a search that the reader would not reproduce.
///
/// It asks `resolve_as_path`, so the repaired link has to name its destination
/// by path and not merely arrive at it. A rewrite that only worked because the
/// resolver falls back to basenames would be a repair whose correctness
/// depended on no other note ever taking that name. Reading is unaffected:
/// `resolve` tries the path first, so anything that lands here lands there.
fn edit_for(
    resolver: &LinkResolver,
    source: &str,
    content: &str,
    link: &RawLink,
    candidate: &str,
) -> Option<Edit> {
    let written = strip_unwritten_extension(&link.path, candidate);
    let target_after = match link.kind {
        Kind::Wiki => normalize_wikilink_target(&written),
        Kind::Markdown => normalize_markdown_path(&written),
    }?;
    if resolver.resolve_as_path(source, &target_after)? != candidate {
        return None;
    }

    Some(Edit {
        byte_start: link.range.start,
        byte_end: link.range.end,
        before: content[link.range.clone()].to_string(),
        after: render(link, &written),
        target_before: link.path.clone(),
        target_after,
        resolves_to: candidate.to_string(),
    })
}

// ---------------------------------------------------------------- scanning --

struct Scanner<'a> {
    config: &'a Config,
    resolver: &'a LinkResolver,
    ignored: &'a IgnoredLinks,
    destinations: &'a Destinations,
    vault: Option<&'a PathBuf>,
}

impl Scanner<'_> {
    fn note(&self, path: &Path) -> Vec<Occurrence> {
        let Ok(content) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        let source = relative_path(&self.config.vault_path, path);

        if is_org(path) {
            return self.org_note(&source, &content);
        }

        let mut found = Vec::new();
        for occurrence in markdown_link_occurrences(path, &content) {
            let link = occurrence.link;
            let target = occurrence.target;

            let resolved = match target.as_deref() {
                Some(target) => self.resolver.resolve(&source, target),
                None => Some(source.clone()),
            };
            match resolved {
                Some(resolved) => {
                    if !link.anchor.is_empty() {
                        if let Some(reason) =
                            self.resolver.missing_anchor_reason(&resolved, &link.anchor)
                        {
                            found.push(self.classify_missing_anchor(
                                &source,
                                &content,
                                &link,
                                target.as_deref(),
                                &resolved,
                                reason,
                            ));
                        }
                    }
                }
                None => {
                    let Some(target) = target else { continue };
                    if !self.ignored.contains(&target) {
                        found.push(self.classify(&source, &content, &link, target));
                    }
                }
            }
        }
        found
    }

    fn classify(&self, source: &str, content: &str, link: &RawLink, target: String) -> Occurrence {
        let reason = reason_for(&target);
        let diagnostic_target = format!("{target}{}", link.anchor);
        let mut occurrence = Occurrence {
            source: source.to_string(),
            line: line_of(content, link.range.start),
            column: Some(column_of(content, link.range.start)),
            syntax: link.kind.name(),
            raw: content[link.range.clone()].to_string(),
            raw_target: format!("{}{}", link.path, link.anchor),
            target: diagnostic_target,
            reason,
            status: UNRESOLVED,
            candidates: Vec::new(),
            edit: None,
            note: None,
        };

        // A date and a citation label name no note, so there is nothing on
        // disk that could settle what they meant.
        if reason == MISSING_DATE || reason == NUMERIC_LABEL {
            occurrence.note = Some(match reason {
                MISSING_DATE => "the target names a date, not a note".into(),
                _ => "the target is a numeric label, not a note".into(),
            });
            return occurrence;
        }

        let (candidates, safe_eligible) =
            candidates_for(self.resolver, self.destinations, source, &target);
        if candidates.is_empty() {
            occurrence.note = Some("no file matches this target".into());
            return occurrence;
        }

        occurrence.status = AMBIGUOUS;
        if !safe_eligible {
            occurrence.note = Some(match candidates[0].basis {
                BASENAME => format!(
                    "only the basename {:?} matches, which is a resemblance rather than a path",
                    candidates[0].matched
                ),
                _ => format!(
                    "{} files end with {:?}",
                    candidates.len(),
                    candidates[0].matched
                ),
            });
            occurrence.candidates = candidates;
            return occurrence;
        }

        let only = candidates[0].path.clone();
        if !inside_real_vault(self.vault, &only) {
            occurrence.note = Some(format!("{only} is not reachable inside the real vault"));
            occurrence.candidates = candidates;
            return occurrence;
        }

        match edit_for(self.resolver, source, content, link, &only) {
            Some(edit) => {
                occurrence.status = SAFE;
                occurrence.edit = Some(edit);
            }
            None => {
                occurrence.note = Some(format!(
                    "a link written as {only} would not resolve back to it"
                ));
            }
        }
        occurrence.candidates = candidates;
        occurrence
    }

    fn classify_missing_anchor(
        &self,
        source: &str,
        content: &str,
        link: &RawLink,
        target: Option<&str>,
        resolved: &str,
        reason: &'static str,
    ) -> Occurrence {
        debug_assert!(reason == MISSING_HEADING || reason == MISSING_BLOCK);
        let target = match target {
            Some(target) => format!("{target}{}", link.anchor),
            None => link.anchor.clone(),
        };
        let noun = if reason == MISSING_HEADING {
            "heading"
        } else {
            "block ID"
        };
        Occurrence {
            source: source.to_string(),
            line: line_of(content, link.range.start),
            column: Some(column_of(content, link.range.start)),
            syntax: link.kind.name(),
            raw: content[link.range.clone()].to_string(),
            raw_target: format!("{}{}", link.path, link.anchor),
            target,
            reason,
            status: UNRESOLVED,
            candidates: Vec::new(),
            edit: None,
            note: Some(format!("the {noun} does not exist in {resolved}")),
        }
    }

    /// org links are reported with their line and never with a repair.
    /// knapper reads org and does not rewrite it, here as in `move` and
    /// `demote`, so proposing an edit would promise something no apply could
    /// keep.
    fn org_note(&self, source: &str, content: &str) -> Vec<Occurrence> {
        org::org_links(content)
            .into_iter()
            .filter(|link| {
                self.resolver.resolve(source, &link.target).is_none()
                    && !self.ignored.contains(&link.target)
            })
            .map(|link| Occurrence {
                source: source.to_string(),
                line: link.line,
                column: None,
                syntax: "org",
                raw: link.raw,
                raw_target: link.raw_target,
                target: link.target,
                reason: ORG_LINK,
                status: UNRESOLVED,
                candidates: Vec::new(),
                edit: None,
                note: Some("knapper reads org links and never rewrites them".into()),
            })
            .collect()
    }
}

/// Every broken link in the vault, as an occurrence with a position.
///
/// Ordered by source, then line, then column, so two runs over an unchanged
/// vault produce byte-identical output.
pub fn scan(config: &Config) -> Vec<Occurrence> {
    let resolver = build_resolver(config);
    let ignored = IgnoredLinks::new(&config.ignore_links);
    let files: BTreeSet<String> = all_files(config)
        .iter()
        .map(|path| relative_path(&config.vault_path, path))
        .collect();
    let destinations = Destinations::build(&files);
    let vault = config.vault_path.canonicalize().ok();

    let scanner = Scanner {
        config,
        resolver: &resolver,
        ignored: &ignored,
        destinations: &destinations,
        vault: vault.as_ref(),
    };

    let mut found: Vec<Occurrence> = all_notes(config)
        .par_iter()
        .flat_map_iter(|path| scanner.note(path))
        .collect();
    found.sort_by(|a, b| a.key().cmp(&b.key()));
    found
}

// --------------------------------------------------------------- reporting --

pub fn counts(occurrences: &[Occurrence]) -> BTreeMap<&'static str, usize> {
    let mut counts: BTreeMap<&'static str, usize> =
        [(SAFE, 0), (AMBIGUOUS, 0), (UNRESOLVED, 0)].into();
    for occurrence in occurrences {
        *counts.entry(occurrence.status).or_insert(0) += 1;
    }
    counts
}

/// One occurrence as JSON. `broken-links` prints these without the proposed
/// edit; `repair-links` prints them with it.
pub fn occurrence_json(occurrence: &Occurrence, with_edit: bool) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("source".into(), json!(occurrence.source));
    out.insert("line".into(), json!(occurrence.line));
    out.insert("column".into(), json!(occurrence.column));
    out.insert("syntax".into(), json!(occurrence.syntax));
    out.insert("raw".into(), json!(occurrence.raw));
    out.insert("raw_target".into(), json!(occurrence.raw_target));
    out.insert("target".into(), json!(occurrence.target));
    out.insert("reason".into(), json!(occurrence.reason));
    out.insert("status".into(), json!(occurrence.status));
    out.insert(
        "candidates".into(),
        json!(occurrence
            .candidates
            .iter()
            .map(|c| json!({"path": c.path, "basis": c.basis, "matched": c.matched}))
            .collect::<Vec<_>>()),
    );
    out.insert("note".into(), json!(occurrence.note));

    if with_edit {
        out.insert(
            "edit".into(),
            match &occurrence.edit {
                Some(edit) => json!({
                    "byte_start": edit.byte_start,
                    "byte_end": edit.byte_end,
                    "before": edit.before,
                    "after": edit.after,
                    "target_before": edit.target_before,
                    "target_after": edit.target_after,
                    "resolves_to": edit.resolves_to,
                }),
                None => Value::Null,
            },
        );
    }
    Value::Object(out)
}

/// `file:line:column`, or `file:line` where there is no column to give.
pub fn position(occurrence: &Occurrence) -> String {
    match occurrence.column {
        Some(column) => format!("{}:{}:{}", occurrence.source, occurrence.line, column),
        None => format!("{}:{}", occurrence.source, occurrence.line),
    }
}

fn why(occurrence: &Occurrence) -> String {
    let (Some(edit), Some(candidate)) = (&occurrence.edit, occurrence.candidates.first()) else {
        return match &occurrence.note {
            Some(note) => format!("{}: {note}", occurrence.reason),
            None => occurrence.reason.to_string(),
        };
    };

    let evidence = match candidate.basis {
        PERCENT_DECODING => format!("decoding the target gives {:?}", candidate.matched),
        _ => format!("the only file whose path ends with {:?}", candidate.matched),
    };
    format!(
        "{} ({}: {evidence}, and {:?} resolves back to it)",
        edit.resolves_to, candidate.basis, edit.target_after
    )
}

fn report(occurrences: &[Occurrence], format: &str) {
    let counts = counts(occurrences);
    let files: BTreeSet<&str> = occurrences.iter().map(|o| o.source.as_str()).collect();

    if format == "json" {
        let mut out = serde_json::Map::new();
        out.insert("kind".into(), json!("repair-links"));
        out.insert("dry_run".into(), json!(true));
        out.insert("applied".into(), json!(false));
        out.insert(
            "summary".into(),
            json!({
                "files": files.len(),
                "occurrences": occurrences.len(),
                "safe": counts[SAFE],
                "ambiguous": counts[AMBIGUOUS],
                "unresolved": counts[UNRESOLVED],
            }),
        );
        out.insert(
            "occurrences".into(),
            json!(occurrences
                .iter()
                .map(|o| occurrence_json(o, true))
                .collect::<Vec<_>>()),
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&Value::Object(out)).unwrap()
        );
        return;
    }

    println!(
        "[DRY RUN] repair-links: {} broken link occurrence(s) in {} file(s)",
        occurrences.len(),
        files.len()
    );
    println!(
        "  safe {}, ambiguous {}, unresolved {}",
        counts[SAFE], counts[AMBIGUOUS], counts[UNRESOLVED]
    );

    for status in [SAFE, AMBIGUOUS, UNRESOLVED] {
        let group: Vec<&Occurrence> = occurrences.iter().filter(|o| o.status == status).collect();
        if group.is_empty() {
            continue;
        }
        println!("\n{status} ({})", group.len());
        for occurrence in group {
            match &occurrence.edit {
                Some(edit) => println!(
                    "  {}  {} -> {}",
                    position(occurrence),
                    edit.before,
                    edit.after
                ),
                None => println!("  {}  {}", position(occurrence), occurrence.raw),
            }
            println!("      {}", why(occurrence));
            if occurrence.status == AMBIGUOUS {
                for candidate in &occurrence.candidates {
                    println!("      - {}", candidate.path);
                }
            }
        }
    }

    println!("\nNothing was written. repair-links plans repairs; it never edits the vault.");
}

// ------------------------------------------------------------- the command --

pub fn repair_links(config: &Config, dry_run: bool, format: &str) -> Result<()> {
    // Belt and braces: the CLI refuses this before a vault is even read, so
    // that omitting the flag can never be mistaken for asking for a write.
    if !dry_run {
        bail!("repair-links only plans repairs. Re-run with --dry-run.");
    }
    report(&scan(config), format);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_or_a_citation_label_is_not_a_missing_note() {
        for target in [
            "2026-07-04",
            "2026/07/04",
            "20260704",
            "2026-07",
            "8/18",
            "2026.07.04",
        ] {
            assert_eq!(reason_for(target), MISSING_DATE, "target: {target}");
        }
        for target in ["12", "[3]", "(7)", "1"] {
            assert_eq!(reason_for(target), NUMERIC_LABEL, "target: {target}");
        }
        assert_eq!(reason_for("Roam"), MISSING_NOTE);
        assert_eq!(reason_for("old/notes/Foo"), MISSING_PATH);
        // A note whose name merely begins with a date is still a note.
        assert_eq!(reason_for("2026-07-04 retro"), MISSING_NOTE);
    }

    fn destinations(files: &[&str]) -> Destinations {
        Destinations::build(&files.iter().map(|f| (*f).to_string()).collect())
    }

    /// The suffix has to agree component by component, from the end. A
    /// prefix, an infix or half a component is not a match.
    #[test]
    fn a_suffix_match_is_whole_components_from_the_end() {
        let index = destinations(&["notes/Foo.md", "assets/paper.pdf", "RoamResearch.md"]);

        let (take, matched, hits) = index.longest_suffix("legacy/notes/Foo.md").unwrap();
        // Comparison folds case; the report echoes what the target said.
        assert_eq!((take, matched), (2, "notes/Foo.md".to_string()));
        assert_eq!(hits, ["notes/Foo.md".to_string()].into());

        // A note is indexed with and without its extension, because a link
        // may be written either way.
        let (take, _, hits) = index.longest_suffix("../../old/notes/Foo").unwrap();
        assert_eq!(take, 2);
        assert_eq!(hits, ["notes/Foo.md".to_string()].into());

        // A leaf file is indexed under its full name only.
        let (_, _, hits) = index.longest_suffix("stale/assets/paper.pdf").unwrap();
        assert_eq!(hits, ["assets/paper.pdf".to_string()].into());

        // A renamed concept is not a suffix of anything.
        assert!(index.longest_suffix("Roam").is_none());
        assert!(index.longest_suffix("notes/Fo").is_none());
    }

    /// A single component is a basename, and a basename is never enough.
    #[test]
    fn a_basename_only_match_is_reported_as_such() {
        let index = destinations(&["notes/Foo.md", "archive/notes/Foo.md"]);
        let (take, _, hits) = index.longest_suffix("Foo").unwrap();
        assert_eq!(take, 1);
        assert_eq!(hits.len(), 2);
    }
}
