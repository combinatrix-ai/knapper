//! Demoting a wikilink target to a tag.
//!
//! A vault that has been written in for years accumulates `[[COO採用]]`:
//! square brackets used as a highlighter, never as a promise that a note by
//! that name exists. knapper is right to call those broken links -- a hard
//! reference to a missing note is a broken link -- and the fix is not to
//! weaken the report but to write what was meant. `#COO採用` is the same
//! label with none of the promise.
//!
//! So this rewrites the *exact* form and nothing else. `[[X]]` becomes `#X`;
//! `[[X|alias]]`, `[[X#heading]]`, `![[X]]` and `[[folder/X]]` do not, because
//! a tag has nowhere to keep an alias, an anchor, an embed or a path. Those
//! are reported rather than mangled, and rather than silently passed over --
//! a caller has to know the ones it still has to deal with by hand.
//!
//! Everything is planned before anything is written, so `--dry-run` shows the
//! real plan, and a write that fails part-way puts back what it touched.

use std::ops::Range;
use std::sync::LazyLock;

use anyhow::{anyhow, bail, Result};
use rayon::prelude::*;
use regex::Regex;
use serde_json::{json, Value};

use crate::note::split_frontmatter;
use crate::parser::{mask_noncontent, strip_anchor};
use crate::topic::is_taggable;
use crate::vault::{all_notes, is_org, relative_path, Config};

// Any `[[...]]` with nothing bracketed inside it. Roam's `[[[[X]]]]` matches
// the inner pair, and is then refused for the brackets around it.
static WIKILINK_SPAN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\[\]]*)\]\]").unwrap());

#[derive(Debug, Default, Clone)]
pub struct Options {
    pub dry_run: bool,
    /// The tag to write, when it cannot be the target itself.
    pub tag: Option<String>,
    /// Demote even though the target names a note that exists.
    pub allow_existing_note: bool,
}

/// Why one occurrence cannot become a tag. Each is a shape a tag has nowhere
/// to put, so rewriting it would lose something the note said.
const ALIAS: &str = "alias";
const ANCHOR: &str = "anchor";
const EMBED: &str = "embed";
const PATH: &str = "path-qualified";
const ADJACENT: &str = "adjacent text";
const FRONTMATTER: &str = "frontmatter";
const ORG: &str = "org-mode";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Skipped {
    file: String,
    line: usize,
    text: String,
    reason: &'static str,
}

#[derive(Debug, Clone)]
struct Change {
    line: usize,
    before: String,
    after: String,
}

#[derive(Debug, Clone)]
struct FileEdit {
    path: String,
    content: String,
    changes: Vec<Change>,
}

#[derive(Debug, Default)]
struct Plan {
    target: String,
    tag: String,
    edits: Vec<FileEdit>,
    skipped: Vec<Skipped>,
}

impl Plan {
    fn links(&self) -> usize {
        self.edits.iter().map(|e| e.changes.len()).sum()
    }
}

// ------------------------------------------------------------- the target --

/// The bare target a demote names, or why it is not one.
///
/// A demote is defined on a *target*, not on a link, so the argument may be
/// written either way: `COO採用` or `[[COO採用]]`. Anything carrying an alias,
/// an anchor or a path is refused here rather than half-understood, because
/// each of them names something a tag cannot express and guessing which part
/// was meant is how a rewrite loses text.
fn parse_target(raw: &str) -> Result<String> {
    let mut target = raw.trim();
    if let Some(inner) = target.strip_prefix("[[").and_then(|t| t.strip_suffix("]]")) {
        target = inner;
    }
    let target = target.trim();

    if target.is_empty() {
        bail!("Nothing to demote: the target is empty.");
    }
    if target.contains('|') {
        bail!("Aliased target: {target:?}\nDemote names a target, not a link. Write the target alone.");
    }
    if target.contains('#') || target.contains('^') {
        bail!("Anchored target: {target:?}\nDemote names a whole note target, not a heading or a block inside one.");
    }
    if target.contains('/') || target.contains('\\') {
        bail!(
            "Path-qualified target: {target:?}\n\
             Demote names a bare wikilink target. To write a nested tag, name the \
             bare target and pass --tag a/b."
        );
    }
    Ok(target.to_string())
}

/// The tag the target becomes.
fn parse_tag(target: &str, requested: Option<&str>) -> Result<String> {
    match requested {
        Some(tag) => {
            let tag = tag.trim().strip_prefix('#').unwrap_or(tag.trim());
            if !is_taggable(tag) {
                bail!(
                    "Not a tag: {:?}\n\
                     A tag is letters, digits, _ and -, with / for nesting and no spaces, \
                     and needs at least one letter.",
                    format!("#{tag}")
                );
            }
            Ok(tag.to_string())
        }
        None if is_taggable(target) => Ok(target.to_string()),
        None => Err(anyhow!(
            "{target:?} cannot be written as a tag: a tag has no spaces, \
             and needs at least one letter.\n\
             Choose the tag yourself with --tag, e.g. --tag {}",
            target.replace(char::is_whitespace, "-")
        )),
    }
}

// --------------------------------------------------------------- planning --

/// Whether a `[[...]]` span refers to this target, and if so what stops it
/// from becoming a tag.
///
/// `None` means the span is about something else entirely and is not this
/// command's business. `Some(None)` means it can be rewritten.
fn classify(
    content: &str,
    span: &Range<usize>,
    inner: &str,
    target: &str,
) -> Option<Option<&'static str>> {
    let (base, aliased) = match inner.split_once('|') {
        Some((base, _)) => (base, true),
        None => (inner, false),
    };
    let anchored = base.trim() != strip_anchor(base);
    let base = strip_anchor(base);

    // Is this span about the target at all? A path-qualified link is a
    // different target, but it is the same *name*, so it is reported rather
    // than passed over in silence.
    let qualified = base.contains('/') || base.contains('\\');
    let names_target = base == target
        || (qualified && base.rsplit(['/', '\\']).next().map(str::trim) == Some(target));
    if !names_target {
        return None;
    }

    let bytes = content.as_bytes();
    let before = span.start.checked_sub(1).map(|i| bytes[i]);
    let after = content[span.end..].chars().next();

    Some(if before == Some(b'!') {
        Some(EMBED)
    } else if aliased {
        Some(ALIAS)
    } else if anchored {
        Some(ANCHOR)
    } else if qualified {
        Some(PATH)
    } else if before.is_some_and(|b| !(b as char).is_whitespace()) {
        // `#X` only reads as a tag at the start of a line or after
        // whitespace, so `see[[X]]` has nowhere to put one.
        Some(ADJACENT)
    } else if after.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '/') {
        // `[[X]]y` would become `#Xy`, which is a different tag.
        Some(ADJACENT)
    } else {
        None
    })
}

fn line_of(content: &str, offset: usize) -> usize {
    content[..offset].matches('\n').count() + 1
}

/// Plan one note: the spans to rewrite, and the ones that cannot be.
fn plan_note(
    config: &Config,
    path: &std::path::Path,
    target: &str,
    tag: &str,
) -> (Option<FileEdit>, Vec<Skipped>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return (None, Vec::new());
    };
    let file = relative_path(&config.vault_path, path);
    let mut skipped = Vec::new();

    // knapper reads org-mode but does not rewrite it, here as everywhere
    // else. An org note that links to the target is reported and left alone.
    //
    // The report still has to be actionable, so the lines come from the file
    // rather than from the parsed link list, which has no positions. A link
    // org writes in some other form -- `file:`, `id:` -- has no line to give,
    // and is reported once against the note.
    if is_org(path) {
        if !crate::org::parse_org(&content)
            .links
            .iter()
            .any(|link| link == target)
        {
            return (None, skipped);
        }
        let written = format!("[[{target}]");
        for (index, line) in content.lines().enumerate() {
            if line.contains(&written) {
                skipped.push(Skipped {
                    file: file.clone(),
                    line: index + 1,
                    text: format!("[[{target}]]"),
                    reason: ORG,
                });
            }
        }
        if skipped.is_empty() {
            skipped.push(Skipped {
                file,
                line: 1,
                text: format!("[[{target}]]"),
                reason: ORG,
            });
        }
        return (None, skipped);
    }

    let (_, body) = split_frontmatter(&content);
    let prefix = content.len() - body.len();

    // A wikilink in frontmatter is a real link -- Obsidian resolves it -- but
    // a YAML value is not prose, and `#X` in one is a string, not a tag.
    for span in WIKILINK_SPAN.captures_iter(&content[..prefix]) {
        let whole = span.get(0).unwrap();
        let range = whole.start()..whole.end();
        if classify(&content, &range, &span[1], target).is_some() {
            skipped.push(Skipped {
                file: file.clone(),
                line: line_of(&content, range.start),
                text: whole.as_str().to_string(),
                reason: FRONTMATTER,
            });
        }
    }

    // Masking keeps every byte offset, so a span found in the masked body is
    // the same span in the file -- which is what makes a splice safe. Code
    // fences, inline spans and %%comments%% hold no references at all, so
    // they are neither rewritten nor reported.
    let masked = mask_noncontent(body);
    let mut spans: Vec<Range<usize>> = Vec::new();
    let mut changes = Vec::new();

    for span in WIKILINK_SPAN.captures_iter(&masked) {
        let whole = span.get(0).unwrap();
        let range = (prefix + whole.start())..(prefix + whole.end());
        let Some(verdict) = classify(&content, &range, &span[1], target) else {
            continue;
        };
        match verdict {
            Some(reason) => skipped.push(Skipped {
                file: file.clone(),
                line: line_of(&content, range.start),
                text: content[range.clone()].to_string(),
                reason,
            }),
            None => {
                changes.push(Change {
                    line: line_of(&content, range.start),
                    before: content[range.clone()].to_string(),
                    after: format!("#{tag}"),
                });
                spans.push(range);
            }
        }
    }

    if spans.is_empty() {
        return (None, skipped);
    }

    // Splice from the end so every earlier offset still stands. Only the
    // span itself is replaced, so indentation, punctuation, the rest of the
    // line and the file's line endings are all left exactly as they were.
    let mut rewritten = content.clone();
    for range in spans.iter().rev() {
        rewritten.replace_range(range.clone(), &format!("#{tag}"));
    }

    (
        Some(FileEdit {
            path: file,
            content: rewritten,
            changes,
        }),
        skipped,
    )
}

fn build_plan(config: &Config, target: &str, tag: &str) -> Plan {
    let planned: Vec<(Option<FileEdit>, Vec<Skipped>)> = all_notes(config)
        .par_iter()
        .map(|path| plan_note(config, path, target, tag))
        .collect();

    let mut plan = Plan {
        target: target.to_string(),
        tag: tag.to_string(),
        ..Default::default()
    };
    for (edit, skipped) in planned {
        plan.edits.extend(edit);
        plan.skipped.extend(skipped);
    }
    plan.edits.sort_by(|a, b| a.path.cmp(&b.path));
    for edit in &mut plan.edits {
        edit.changes.sort_by_key(|c| c.line);
    }
    plan.skipped.sort();
    plan
}

// ---------------------------------------------------------------- writing --

/// Write the plan, putting back whatever was already written if one write
/// fails. A half-demoted vault is worse than an undemoted one: the links that
/// changed and the ones that did not look identical afterwards.
fn apply(config: &Config, plan: &Plan) -> Result<()> {
    let mut written: Vec<(&str, String)> = Vec::new();

    for edit in &plan.edits {
        let path = config.vault_path.join(&edit.path);
        let outcome = std::fs::read_to_string(&path)
            .and_then(|original| std::fs::write(&path, &edit.content).map(|()| original));

        let err = match outcome {
            Ok(original) => {
                written.push((&edit.path, original));
                continue;
            }
            Err(err) => err,
        };

        // What the rollback could not undo has to be named. "Nothing was
        // changed" when something was is the one report that leaves a vault
        // in a state nobody can reason about.
        let stuck: Vec<&str> = written
            .iter()
            .filter(|(done, before)| std::fs::write(config.vault_path.join(done), before).is_err())
            .map(|(done, _)| *done)
            .collect();

        return Err(match stuck.is_empty() {
            true => anyhow!("Could not write {}: {err}. Nothing was changed.", edit.path),
            false => anyhow!(
                "Could not write {}: {err}. The rollback was incomplete, \
                 and these notes are still demoted: {}",
                edit.path,
                stuck.join(", ")
            ),
        });
    }
    Ok(())
}

// --------------------------------------------------------------- reporting --

fn report(plan: &Plan, dry_run: bool, format: &str) {
    let files: Vec<&String> = plan.edits.iter().map(|e| &e.path).collect();

    if format == "json" {
        let mut out = serde_json::Map::new();
        out.insert("kind".into(), json!("demote"));
        out.insert("target".into(), json!(plan.target));
        out.insert("tag".into(), json!(plan.tag));
        out.insert("dry_run".into(), json!(dry_run));
        out.insert("applied".into(), json!(!dry_run));
        out.insert("files_updated".into(), json!(files));
        out.insert("links_updated".into(), json!(plan.links()));
        out.insert(
            "edits".into(),
            json!(plan
                .edits
                .iter()
                .map(|e| json!({
                    "file": e.path,
                    "links": e.changes.len(),
                    "changes": e.changes.iter().map(|c| json!({
                        "line": c.line, "before": c.before, "after": c.after,
                    })).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>()),
        );
        out.insert(
            "skipped".into(),
            json!(plan
                .skipped
                .iter()
                .map(|s| json!({
                    "file": s.file, "line": s.line, "text": s.text, "reason": s.reason,
                }))
                .collect::<Vec<_>>()),
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&Value::Object(out)).unwrap()
        );
        return;
    }

    let lead = if dry_run { "[DRY RUN] " } else { "" };
    println!("{lead}Demoting [[{}]] -> #{}", plan.target, plan.tag);

    if dry_run {
        println!(
            "  Would update {} links in {} files:",
            plan.links(),
            plan.edits.len()
        );
        for edit in &plan.edits {
            println!("    {}", edit.path);
            for change in &edit.changes {
                println!(
                    "      {}: {} -> {}",
                    change.line, change.before, change.after
                );
            }
        }
    } else {
        for edit in &plan.edits {
            println!("  Updated {} links in {}", edit.changes.len(), edit.path);
        }
    }

    for entry in &plan.skipped {
        println!(
            "  ⚠️ {}:{}: {} cannot be demoted ({})",
            entry.file, entry.line, entry.text, entry.reason
        );
    }

    if dry_run {
        println!("\nNothing was written.");
    } else {
        println!(
            "\nDone! Demoted {} links in {} files.",
            plan.links(),
            plan.edits.len()
        );
    }
}

// ------------------------------------------------------------- the command --

pub fn demote(config: &Config, target_arg: &str, options: &Options, format: &str) -> Result<()> {
    let target = parse_target(target_arg)?;
    let tag = parse_tag(&target, options.tag.as_deref())?;

    let plan = build_plan(config, &target, &tag);

    if plan.edits.is_empty() && plan.skipped.is_empty() {
        bail!("No [[{target}]] wikilinks found.");
    }

    // Demoting a link to a note that exists is not a relabelling, it is a
    // deletion: the reference stops pointing anywhere and the note loses a
    // backlink. That is a real thing to want, and never a thing to do by
    // accident, so it has to be asked for.
    if !options.allow_existing_note {
        if let Some(found) = crate::graph::build_resolver(config).resolve("", &target) {
            bail!(
                "[[{target}]] resolves to {found}, so it is a real reference, not a label.\n\
                 Demoting it would drop the link to that note. Rerun with \
                 --allow-existing-note to do it anyway."
            );
        }
    }

    if !options.dry_run {
        apply(config, &plan)?;
    }
    report(&plan, options.dry_run, format);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(raw: &str) -> String {
        parse_target(raw).unwrap()
    }

    /// The argument may be written either way, because both are what a reader
    /// has in front of them when they decide to demote something.
    #[test]
    fn a_target_may_be_written_bare_or_bracketed() {
        assert_eq!(target("COO採用"), "COO採用");
        assert_eq!(target("[[COO採用]]"), "COO採用");
        assert_eq!(target("  [[COO採用]]  "), "COO採用");
    }

    /// Every refusal here names something a tag cannot hold. Guessing which
    /// part was meant is how a rewrite loses text.
    #[test]
    fn ambiguous_targets_are_refused_rather_than_guessed() {
        for raw in [
            "",
            "   ",
            "[[]]",
            "COO採用|採用",
            "COO採用#面接",
            "COO採用^b12",
            "folder/COO採用",
            "folder\\COO採用",
        ] {
            assert!(parse_target(raw).is_err(), "should refuse: {raw:?}");
        }
    }

    #[test]
    fn the_tag_is_the_target_unless_it_cannot_be() {
        assert_eq!(parse_tag("COO採用", None).unwrap(), "COO採用");
        assert_eq!(parse_tag("work", None).unwrap(), "work");

        // A target with a space is not a tag, and the refusal says what to do.
        let err = parse_tag("Daily Tasks", None).unwrap_err().to_string();
        assert!(err.contains("--tag"), "{err}");
        assert!(err.contains("Daily-Tasks"), "{err}");

        assert_eq!(
            parse_tag("Daily Tasks", Some("Daily-Tasks")).unwrap(),
            "Daily-Tasks"
        );
        // A leading # is how a tag is written, so it is accepted and dropped.
        assert_eq!(
            parse_tag("Daily Tasks", Some("#work/daily")).unwrap(),
            "work/daily"
        );
        assert!(parse_tag("Daily Tasks", Some("still spaced")).is_err());
    }

    /// The classifier is the whole safety argument, so it is checked on the
    /// forms directly rather than through a vault.
    fn verdict(content: &str, target: &str) -> Vec<Option<&'static str>> {
        WIKILINK_SPAN
            .captures_iter(content)
            .filter_map(|c| {
                let whole = c.get(0).unwrap();
                classify(content, &(whole.start()..whole.end()), &c[1], target)
            })
            .collect()
    }

    #[test]
    fn an_exact_wikilink_is_the_only_thing_demoted() {
        for content in [
            "[[X]]",
            "see [[X]]",
            "see [[X]] here",
            "- [[X]]\n",
            "[[X]], and more",
            // Japanese punctuation ends a tag as cleanly as ASCII does.
            "面接の準備 [[X]]。",
            "面接の準備 [[X]]、続き",
        ] {
            assert_eq!(verdict(content, "X"), [None], "input: {content:?}");
        }
    }

    #[test]
    fn every_lossy_form_is_reported_rather_than_rewritten() {
        for (content, reason) in [
            ("[[X|alias]]", ALIAS),
            ("[[X#Heading]]", ANCHOR),
            ("[[X^b12]]", ANCHOR),
            ("![[X]]", EMBED),
            ("[[folder/X]]", PATH),
            ("[[folder/X|alias]]", ALIAS),
            ("see[[X]]", ADJACENT),
            ("[[X]]tail", ADJACENT),
            ("[[[[X]]]]", ADJACENT),
            // `#X` reads as a tag only at the start of a line or after
            // whitespace, so an opening bracket in front of it -- fullwidth
            // or not -- leaves nowhere to put one.
            ("（[[X]]）", ADJACENT),
            ("([[X]])", ADJACENT),
            // And a word character after it would be swallowed into the tag:
            // `#Xの` is a different tag from `#X`.
            ("見た [[X]]の進捗", ADJACENT),
        ] {
            assert_eq!(verdict(content, "X"), [Some(reason)], "input: {content:?}");
        }
    }

    #[test]
    fn links_to_other_targets_are_not_this_commands_business() {
        for content in ["[[Y]]", "[[XY]]", "[[X Y]]", "[[folder/Y]]", "[[Y|X]]"] {
            assert!(verdict(content, "X").is_empty(), "input: {content:?}");
        }
    }

    /// Matching is exact: `[[coo採用]]` is not `[[COO採用]]`, because a demote
    /// that folded case would rewrite links the user never named.
    #[test]
    fn matching_is_exact_and_case_sensitive() {
        assert_eq!(verdict("[[COO採用]]", "COO採用"), [None]);
        assert!(verdict("[[coo採用]]", "COO採用").is_empty());
        assert!(verdict("[[COO採用計画]]", "COO採用").is_empty());
    }
}
