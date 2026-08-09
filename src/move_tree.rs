//! Moving a whole directory, with every inbound link following it.
//!
//! The single-note `move` rewrites by stem: it finds the text that looks like
//! a link to one name and edits it. That cannot be stretched to a directory,
//! where "a link into this subtree" is a question about resolution rather
//! than about spelling -- a bare `[[README]]` may or may not be one, and the
//! answer depends on where the link is written.
//!
//! So this works the other way round. It resolves every local link in the
//! vault to the file it currently points at, decides what that file's path
//! will be afterwards, and asks whether the link as written would still land
//! there. Only when the answer is no does it edit, and the edit is a span
//! replacement in the file's own bytes, so labels, anchors, titles and
//! percent-encoding survive untouched.
//!
//! Everything is computed before anything is written: the whole plan --
//! which files move, which notes change, and to what -- exists in memory
//! first. A write that fails part-way restores what it already touched and
//! puts the directory back.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

use anyhow::{anyhow, bail, Result};
use percent_encoding::percent_decode_str;
use regex::Regex;
use serde_json::{json, Value};
use walkdir::WalkDir;

use crate::note::{parse_note, split_frontmatter};
use crate::parser::mask_noncontent;
use crate::vault::{
    all_notes, is_excluded, is_org, relative_path, Config, CONFIG_FILENAME, DEFAULT_EXTENSIONS,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct Options {
    pub dry_run: bool,
    /// Move even though inbound org links will be left pointing nowhere.
    pub allow_broken_org_links: bool,
}

/// True when this `move` argument names a directory rather than a note.
///
/// A symlink to a directory answers yes, so that the move can refuse it by
/// name instead of falling through to the note path and reporting that no
/// note was found.
pub fn looks_like_directory(config: &Config, source: &str) -> bool {
    let trimmed = source.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return false;
    }
    let path = Path::new(trimmed);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        config.vault_path.join(path)
    };
    path.symlink_metadata()
        .map(|m| m.is_dir() || (m.file_type().is_symlink() && path.is_dir()))
        .unwrap_or(false)
}

// ------------------------------------------------------------------ paths --

fn dir_of(relative: &str) -> &str {
    match relative.rfind('/') {
        Some(index) => &relative[..index],
        None => "",
    }
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Fold away `.` and `..` textually. `None` means the path climbs out of the
/// vault, which is never something to act on.
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// The path of `to` as written from inside `from_dir`.
fn relative_from(from_dir: &str, to: &str) -> String {
    let from: Vec<&str> = if from_dir.is_empty() {
        Vec::new()
    } else {
        from_dir.split('/').collect()
    };
    let to_parts: Vec<&str> = to.split('/').collect();
    let common = from
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut out: Vec<&str> = vec![".."; from.len() - common];
    out.extend(&to_parts[common..]);
    out.join("/")
}

/// How many leading directory components two paths share.
fn shared_depth(a: &str, b: &str) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    a.split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y)
        .count()
}

fn depth(relative: &str) -> usize {
    relative.split('/').count()
}

/// Where `path` ends up once `source` has been moved to `dest`.
fn remap(path: &str, source: &str, dest: &str) -> String {
    if path == source {
        return dest.to_string();
    }
    match path.strip_prefix(&format!("{source}/")) {
        Some(rest) => format!("{dest}/{rest}"),
        None => path.to_string(),
    }
}

fn inside(path: &str, source: &str) -> bool {
    path == source || path.starts_with(&format!("{source}/"))
}

fn is_note_path(relative: &str) -> bool {
    Path::new(relative)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| DEFAULT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Turn a user-supplied path into a vault-relative one, or refuse it.
fn vault_relative(config: &Config, raw: &str) -> Result<String> {
    let trimmed = raw.trim().replace('\\', "/");
    let trimmed = trimmed.trim_end_matches('/');
    let path = Path::new(trimmed);

    let relative = if path.is_absolute() {
        let vault = config
            .vault_path
            .canonicalize()
            .unwrap_or_else(|_| config.vault_path.clone());
        let here = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        match here.strip_prefix(&vault) {
            Ok(rest) => rest.to_string_lossy().replace('\\', "/"),
            Err(_) => bail!("Outside the vault: {raw}"),
        }
    } else {
        trimmed.to_string()
    };

    normalize(&relative).ok_or_else(|| anyhow!("Outside the vault: {raw}"))
}

/// Refuse a path that is only vault-relative on paper.
///
/// `normalize` folds `..` textually, which says nothing about what is on
/// disk. A symlink called `Elsewhere` pointing at `../outside` makes
/// `Elsewhere/Guide` look like an ordinary relative path, and `rename` would
/// follow it straight out of the vault -- taking the directory with it and
/// leaving every rewritten link pointing at a path that no longer exists
/// inside the vault at all.
///
/// So every component that exists is checked in turn, and the deepest one
/// that does has to canonicalize back inside the canonical vault. Components
/// that do not exist yet are the ones this move will create, underneath a
/// parent already proven to be inside.
fn ensure_inside_vault(config: &Config, relative: &str, what: &str) -> Result<()> {
    let vault = config.vault_path.canonicalize().map_err(|err| {
        anyhow!(
            "Cannot resolve the vault at {}: {err}",
            config.vault_path.display()
        )
    })?;

    let mut deepest = vault.clone();
    let mut current = vault.clone();
    for part in relative.split('/').filter(|p| !p.is_empty()) {
        current = current.join(part);
        match current.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => bail!(
                "{what} passes through a symlink: {relative}\n\
                 knapper moves directories only within the real vault tree, \
                 because a symlink can lead anywhere."
            ),
            Ok(_) => deepest = current.clone(),
            // Nothing here yet, so nothing below it exists either. What gets
            // created will land under `deepest`, which is checked below.
            Err(_) => break,
        }
    }

    let real = deepest
        .canonicalize()
        .map_err(|err| anyhow!("Cannot resolve {}: {err}", deepest.display()))?;
    if !real.starts_with(&vault) {
        bail!("{what} resolves outside the vault: {relative}");
    }
    Ok(())
}

// --------------------------------------------------------------- scanning --

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Wiki,
    Markdown,
}

/// One local link, as written, with the span it occupies in the file.
#[derive(Debug, Clone)]
struct RawLink {
    range: Range<usize>,
    kind: Kind,
    /// `![[..]]` or `![](..)`.
    embed: bool,
    /// The markdown label, verbatim.
    label: String,
    /// The path part, percent-decoded, with no anchor.
    path: String,
    /// `#heading` or `^block-id`, verbatim and still encoded.
    anchor: String,
    /// A wikilink's `|alias`, bar included.
    alias: String,
    /// A markdown link's ` "title"`, leading space included.
    title: String,
    /// The href was written as `<...>`.
    angle: bool,
}

// [[target#anchor|alias]], optionally embedded.
static WIKI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(!)?\[\[([^\[\]|#^]+)((?:#|\^)[^\[\]|]*)?(\|[^\[\]]*)?\]\]").unwrap()
});

// [label](target "title"), optionally embedded, target optionally in <>.
static MD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(!)?\[([^\[\]]*)\]\(\s*(?:<([^>]*)>|([^)\s]+))((?:[ \t]+"[^"]*")?)[ \t]*\)"#)
        .unwrap()
});

static EXTERNAL_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z][a-zA-Z0-9+.\-]*:").unwrap());

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
fn scan_links(content: &str) -> Vec<RawLink> {
    let (_, body) = split_frontmatter(content);
    let start = body_offset(content, body);
    let masked = format!("{}{}", &content[..start], mask_noncontent(body));
    debug_assert_eq!(masked.len(), content.len());

    let mut links: Vec<RawLink> = Vec::new();

    for c in WIKI.captures_iter(&masked) {
        let whole = c.get(0).unwrap();
        links.push(RawLink {
            range: whole.range(),
            kind: Kind::Wiki,
            embed: c.get(1).is_some(),
            label: String::new(),
            path: c[2].trim().to_string(),
            anchor: c.get(3).map(|m| m.as_str().to_string()).unwrap_or_default(),
            alias: c.get(4).map(|m| m.as_str().to_string()).unwrap_or_default(),
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
        if EXTERNAL_SCHEME.is_match(href) || href.starts_with('#') || href.starts_with("//") {
            continue;
        }
        // Obsidian writes a block reference as Note.md#^id, so the anchor
        // starts at the first '#' and everything before it is the path.
        let (path, anchor) = match href.find('#') {
            Some(index) if index > 0 => (&href[..index], &href[index..]),
            _ => (href, ""),
        };
        if path.is_empty() {
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

fn encode_href_path(path: &str) -> String {
    percent_encode(path, true)
}

fn encode_angle_path(path: &str) -> String {
    percent_encode(path, false)
}

fn render(link: &RawLink, path: &str) -> String {
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

// ------------------------------------------------------------- resolution --

/// What a link target resolves to, built once so resolving is a lookup.
///
/// This is deliberately not `LinkResolver`: that one answers "which note is
/// called this?" for the graph, where a bare name has one answer for the
/// whole vault. Rewriting needs "which file does *this* link, written *here*,
/// point at?", which is referrer-relative -- and it needs attachments, since
/// an image inside a moved directory has to keep resolving too.
///
/// The divergence is deliberate for now and known: `graph::LinkResolver`
/// picks a bare name's target by first-seen path, this picks it by proximity
/// to the referrer, so the two can disagree about which of two same-named
/// notes a link means. That is a real inconsistency, not a design -- the
/// graph should eventually learn the referrer-aware rule and both should come
/// from one implementation. Unifying them changes what `backlinks`, `links`
/// and `broken-links` report, so it is its own change with its own contract
/// cases, not a rider on the directory move.
struct Index {
    files: BTreeSet<String>,
    lower: BTreeMap<String, String>,
    by_name: BTreeMap<String, Vec<String>>,
    by_stem: BTreeMap<String, Vec<String>>,
    by_alias: BTreeMap<String, String>,
}

impl Index {
    fn build(files: &[String], aliases: &BTreeMap<String, String>) -> Self {
        let mut lower = BTreeMap::new();
        let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut by_stem: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for file in files {
            lower.entry(file.to_lowercase()).or_insert(file.clone());
            let path = Path::new(file);
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                by_name
                    .entry(name.to_lowercase())
                    .or_default()
                    .push(file.clone());
            }
            if is_note_path(file) {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    by_stem
                        .entry(stem.to_lowercase())
                        .or_default()
                        .push(file.clone());
                }
            }
        }

        Self {
            files: files.iter().cloned().collect(),
            lower,
            by_name,
            by_stem,
            by_alias: aliases
                .iter()
                .map(|(a, p)| (a.to_lowercase(), p.clone()))
                .collect(),
        }
    }

    /// The same index as it will be once the move has happened.
    fn remapped(&self, source: &str, dest: &str) -> Self {
        let files: Vec<String> = self
            .files
            .iter()
            .map(|f| remap(f, source, dest))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let aliases = self
            .by_alias
            .iter()
            .map(|(a, p)| (a.clone(), remap(p, source, dest)))
            .collect();
        Index::build(&files, &aliases)
    }

    /// An existing file at this vault-relative path, trying note extensions
    /// and then a case-insensitive match, as Obsidian does.
    fn lookup(&self, path: &str) -> Option<String> {
        if path.is_empty() {
            return None;
        }
        if self.files.contains(path) {
            return Some(path.to_string());
        }
        for ext in DEFAULT_EXTENSIONS {
            let candidate = format!("{path}.{ext}");
            if self.files.contains(&candidate) {
                return Some(candidate);
            }
        }
        let lower = path.to_lowercase();
        if let Some(hit) = self.lower.get(&lower) {
            return Some(hit.clone());
        }
        for ext in DEFAULT_EXTENSIONS {
            if let Some(hit) = self.lower.get(&format!("{lower}.{ext}")) {
                return Some(hit.clone());
            }
        }
        None
    }

    /// Of several files with the same name, the one a link written in
    /// `referrer` means: the closest, then the shallowest, then the first by
    /// name. This is what keeps two READMEs apart.
    fn nearest(&self, referrer: &str, candidates: &[String]) -> Option<String> {
        let dir = dir_of(referrer);
        candidates
            .iter()
            .min_by_key(|candidate| {
                (
                    std::cmp::Reverse(shared_depth(dir, dir_of(candidate))),
                    depth(candidate),
                    (*candidate).clone(),
                )
            })
            .cloned()
    }

    /// A wikilink target: a vault path, else the nearest file of that name,
    /// else an alias.
    fn resolve_wiki(&self, referrer: &str, target: &str) -> Option<String> {
        let target = target.trim().replace('\\', "/");
        if target.is_empty() {
            return None;
        }

        if target.contains('/') {
            if let Some(hit) = normalize(&target).and_then(|p| self.lookup(&p)) {
                return Some(hit);
            }
            if let Some(hit) =
                normalize(&join_rel(dir_of(referrer), &target)).and_then(|p| self.lookup(&p))
            {
                return Some(hit);
            }
        }

        let lower = target.to_lowercase();
        for table in [&self.by_name, &self.by_stem] {
            if let Some(candidates) = table.get(&lower) {
                if let Some(hit) = self.nearest(referrer, candidates) {
                    return Some(hit);
                }
            }
        }
        self.by_alias.get(&lower).cloned()
    }

    /// A markdown href. The bool says the link was read relative to the note
    /// holding it, which is the only case where moving that note changes what
    /// it has to say.
    ///
    /// There is no name fallback here on purpose: an inline link that names
    /// no existing path is already broken, and guessing at what it meant
    /// would let a move rewrite something it does not understand.
    fn resolve_markdown(&self, referrer: &str, target: &str) -> Option<(String, bool)> {
        let target = target.trim().replace('\\', "/");
        if target.is_empty() {
            return None;
        }
        if let Some(hit) =
            normalize(&join_rel(dir_of(referrer), &target)).and_then(|p| self.lookup(&p))
        {
            return Some((hit, true));
        }
        if !target.starts_with("./") && !target.starts_with("../") {
            if let Some(hit) = normalize(&target).and_then(|p| self.lookup(&p)) {
                return Some((hit, false));
            }
        }
        None
    }

    /// An org link target, which may be written either way.
    fn resolve_org(&self, referrer: &str, target: &str) -> Option<String> {
        if target.starts_with("id:") || target.starts_with('*') {
            // Location-independent: an org ID and a cross-file heading both
            // survive a move untouched.
            return None;
        }
        if let Some((hit, _)) = self.resolve_markdown(referrer, target) {
            return Some(hit);
        }
        self.resolve_wiki(referrer, target)
    }
}

// -------------------------------------------------------------- the plan --

#[derive(Debug, Clone)]
struct Change {
    line: usize,
    before: String,
    after: String,
}

#[derive(Debug, Clone)]
struct Edit {
    /// Where the note is now.
    path: String,
    /// Where it will be when the write happens.
    new_path: String,
    original: String,
    updated: String,
    changes: Vec<Change>,
}

#[derive(Debug, Clone)]
struct Unsupported {
    file: String,
    link: String,
    reason: String,
}

struct Plan {
    source: String,
    destination: String,
    entries: Vec<String>,
    notes: usize,
    edits: Vec<Edit>,
    unsupported: Vec<Unsupported>,
    warnings: Vec<String>,
}

impl Plan {
    fn links_updated(&self) -> usize {
        self.edits.iter().map(|e| e.changes.len()).sum()
    }
}

fn line_of(content: &str, offset: usize) -> usize {
    content[..offset].matches('\n').count() + 1
}

/// Every file under `root`, hidden ones included. This is a walk rather than
/// a read of `all_notes`, because what moves is the physical subtree: the
/// JSONL sidecar, the image, the dotfile.
fn walk_subtree(vault: &Path, root: &Path) -> Vec<String> {
    let mut files: Vec<String> = WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_symlink() || e.path().is_file())
        .map(|e| relative_path(vault, e.path()))
        .collect();
    files.sort();
    files
}

/// Every file in the vault, for resolution. Attachments count; a link can
/// point at one. `.git` and `.obsidian` do not.
fn walk_vault(vault: &Path) -> Vec<String> {
    let mut files: Vec<String> = WalkDir::new(vault)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !matches!(
                    e.file_name().to_str(),
                    Some(".git") | Some(".obsidian") | Some(".trash")
                )
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| relative_path(vault, e.path()))
        .filter(|r| r != CONFIG_FILENAME)
        .collect();
    files.sort();
    files
}

fn build_plan(config: &Config, source: &str, destination: &str) -> Result<Plan> {
    let vault = &config.vault_path;
    let entries = walk_subtree(vault, &vault.join(source));

    // Reading comes first and completely: a note that cannot be read is a
    // note whose links cannot be checked, and finding that out half way
    // through the writes is exactly what this design exists to prevent.
    let mut docs: Vec<(String, String, bool)> = Vec::new();
    for path in all_notes(config) {
        let relative = relative_path(vault, &path);
        match std::fs::read_to_string(&path) {
            Ok(content) => docs.push((relative, content, is_org(&path))),
            Err(err) => bail!(
                "Cannot read {relative}: {err}\n\
                 Every note has to be readable before a directory move starts. \
                 Fix it, or add it to `exclude` in {CONFIG_FILENAME}."
            ),
        }
    }

    // Aliases have to be known before anything resolves: `[[an alias]]` names
    // the note declaring it, wherever that note lives. `parse_note` dispatches
    // on extension, so an org file's aliases arrive the same way.
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    for (relative, content, _) in &docs {
        for alias in parse_note(Path::new(relative), content).aliases {
            aliases.entry(alias).or_insert_with(|| relative.clone());
        }
    }

    let index = Index::build(&walk_vault(vault), &aliases);
    let after = index.remapped(source, destination);

    let mut edits = Vec::new();
    let mut unsupported = Vec::new();

    for (relative, content, org) in &docs {
        let new_relative = remap(relative, source, destination);

        if *org {
            for target in crate::org::extract_org_links(content) {
                let Some(old_target) = index.resolve_org(relative, &target) else {
                    continue;
                };
                let new_target = remap(&old_target, source, destination);
                if new_target == old_target && new_relative == *relative {
                    continue;
                }
                if after.resolve_org(&new_relative, &target).as_deref() != Some(new_target.as_str())
                {
                    unsupported.push(Unsupported {
                        file: relative.clone(),
                        link: target.clone(),
                        reason: format!("org links are not rewritten; it points at {old_target}"),
                    });
                }
            }
            continue;
        }

        let mut changes = Vec::new();
        let mut spans: Vec<(Range<usize>, String)> = Vec::new();

        for link in scan_links(content) {
            let Some(replacement) = rewrite(
                &index,
                &after,
                relative,
                &new_relative,
                &link,
                source,
                destination,
            ) else {
                continue;
            };
            let before = content[link.range.clone()].to_string();
            if replacement == before {
                continue;
            }
            changes.push(Change {
                line: line_of(content, link.range.start),
                before,
                after: replacement.clone(),
            });
            spans.push((link.range, replacement));
        }

        if spans.is_empty() {
            continue;
        }

        let mut updated = content.clone();
        for (range, replacement) in spans.into_iter().rev() {
            updated.replace_range(range, &replacement);
        }
        edits.push(Edit {
            path: relative.clone(),
            new_path: new_relative,
            original: content.clone(),
            updated,
            changes,
        });
    }

    edits.sort_by(|a, b| a.new_path.cmp(&b.new_path));
    unsupported.sort_by(|a, b| (&a.file, &a.link).cmp(&(&b.file, &b.link)));

    // Notes the scanner never saw are notes whose links nobody checked, and
    // saying so is the difference between a partial job and a silent one.
    let scanned: BTreeSet<&String> = docs.iter().map(|(r, _, _)| r).collect();
    let unscanned = entries
        .iter()
        .filter(|e| is_note_path(e) && !scanned.contains(e))
        .count();
    let mut warnings = Vec::new();
    if unscanned > 0 {
        warnings.push(format!(
            "{unscanned} note(s) inside {source} are hidden or excluded; they move, \
             but their own outgoing links were not checked"
        ));
    }

    let notes = entries.iter().filter(|e| is_note_path(e)).count();
    Ok(Plan {
        source: source.to_string(),
        destination: destination.to_string(),
        entries,
        notes,
        edits,
        unsupported,
        warnings,
    })
}

/// What one link must say afterwards, or `None` to leave it exactly as it is.
fn rewrite(
    before: &Index,
    after: &Index,
    referrer: &str,
    new_referrer: &str,
    link: &RawLink,
    source: &str,
    destination: &str,
) -> Option<String> {
    match link.kind {
        Kind::Wiki => {
            let old_target = before.resolve_wiki(referrer, &link.path)?;
            let new_target = remap(&old_target, source, destination);
            if new_target == old_target && new_referrer == referrer {
                return None;
            }
            // A wikilink names a note rather than a path, so most of them
            // still find it. Only rewrite the ones that would not.
            if after.resolve_wiki(new_referrer, &link.path).as_deref() == Some(new_target.as_str())
            {
                return None;
            }
            Some(render(
                link,
                &strip_unwritten_extension(&link.path, &new_target),
            ))
        }
        Kind::Markdown => {
            let (old_target, relative) = before.resolve_markdown(referrer, &link.path)?;
            let new_target = remap(&old_target, source, destination);
            let moved_referrer = new_referrer != referrer;
            if !relative && new_target == old_target {
                return None;
            }
            if relative && new_target == old_target && !moved_referrer {
                return None;
            }

            let mut path = if relative {
                relative_from(dir_of(new_referrer), &new_target)
            } else {
                new_target.clone()
            };
            if link.path.starts_with("./") && !path.starts_with("..") {
                path = format!("./{path}");
            }
            Some(render(link, &strip_unwritten_extension(&link.path, &path)))
        }
    }
}

/// Keep the target spelled the way it was: with its extension if it had one,
/// without if it did not. A vault where notes are linked as `[[Note]]` should
/// not end up with one `[[Archive/Note.md]]` in it.
fn strip_unwritten_extension(written: &str, target: &str) -> String {
    let extension = Path::new(target)
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

// ------------------------------------------------------------- applying --

/// Create the directories that are missing, deepest last, and say which ones
/// were actually created so a rollback can take them away again.
fn create_dirs(path: &Path) -> Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    while !current.exists() {
        missing.push(current.clone());
        if !current.pop() {
            break;
        }
    }
    missing.reverse();
    for dir in &missing {
        std::fs::create_dir(dir)?;
    }
    Ok(missing)
}

/// Remove the directories this move created, reporting the ones that would
/// not go. A directory that is no longer there is not a failure.
fn remove_dirs(dirs: &[PathBuf]) -> Vec<String> {
    let mut failures = Vec::new();
    for dir in dirs.iter().rev() {
        if let Err(err) = std::fs::remove_dir(dir) {
            if dir.exists() {
                failures.push(format!("{} was left behind ({err})", dir.display()));
            }
        }
    }
    failures
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// An exclusively created sibling temp file, and the path it was created at.
///
/// The exclusivity is the point. A fixed name -- `.note.md.knapper-tmp` --
/// races two knapper runs in one vault against each other, and worse, it
/// opens whatever is already at that name: a file a user or another tool put
/// there would be overwritten with note content and then unlinked by the
/// rename. `create_new` fails instead, and the loop moves to the next name.
fn create_temp(path: &Path) -> std::io::Result<(PathBuf, std::fs::File)> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "note".into());
    let pid = std::process::id();

    for _ in 0..1_000 {
        let ticket = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = dir.join(format!(".{name}.knapper-{pid}-{ticket}.tmp"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            // Somebody holds this name. It is not ours to touch: try another.
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("no free temp name next to {}", path.display()),
    ))
}

/// The file a rewrite of `path` must actually replace.
///
/// A symlinked note is still a note: `all_notes` follows the link to decide
/// that, deliberately, and `rename` and single-note `move` write through one
/// because `fs::write` follows symlinks. An atomic rewrite has to do the same
/// or it is not the same operation -- renaming a temp file over the link
/// *replaces the link with a regular file*, quietly detaching a note the user
/// had deliberately shared into the vault.
///
/// So the link is resolved and the file it names is what gets replaced. The
/// link itself is never touched, and still points where it did. A target
/// outside the vault is followed too, for the same reason the rest of knapper
/// follows one: the user put it there on purpose.
fn write_target(path: &Path) -> std::io::Result<PathBuf> {
    match path.symlink_metadata() {
        // A dangling link has no file to rewrite. Reporting it here stops the
        // move before this note is touched, rather than after.
        Ok(meta) if meta.file_type().is_symlink() => path.canonicalize(),
        _ => Ok(path.to_path_buf()),
    }
}

/// Write through a sibling temp file, so a note is never left half-written.
///
/// The note's own permissions come across before the rename: a note somebody
/// made read-only, or group-writable for a shared vault, should still be that
/// after knapper rewrites a link in it, and a fresh temp file starts from
/// this process's umask instead.
fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;

    let resolved = write_target(path)?;
    let path = resolved.as_path();

    let (temp, mut file) = create_temp(path)?;

    let written = file
        .write_all(content.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);

    let finish = written
        .and_then(|()| match std::fs::metadata(path) {
            Ok(meta) => std::fs::set_permissions(&temp, meta.permissions()),
            // No target yet means no permissions to carry over.
            Err(_) => Ok(()),
        })
        .and_then(|()| std::fs::rename(&temp, path));

    if let Err(err) = finish {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    Ok(())
}

/// Undo a partly applied plan, and say what could not be undone.
///
/// Every step is attempted even after one fails: stopping at the first
/// problem would leave more behind, not less. The failures are returned
/// rather than swallowed, because "the move was rolled back" is a claim
/// about the vault, and it has to be true when it is made.
fn roll_back(
    written: &[(PathBuf, String)],
    to: &Path,
    from: &Path,
    created: &[PathBuf],
) -> Vec<String> {
    let mut failures = Vec::new();

    for (path, original) in written.iter().rev() {
        // `fs::write` follows a symlink, which is what put the content there
        // in the first place, so a symlinked note is restored through the
        // link rather than replaced by one.
        if let Err(err) = std::fs::write(path, original) {
            failures.push(format!("{} is still rewritten ({err})", path.display()));
        }
    }
    if let Err(err) = std::fs::rename(to, from) {
        failures.push(format!(
            "{} is still at {} ({err})",
            from.display(),
            to.display()
        ));
    }
    failures.extend(remove_dirs(created));
    failures
}

#[cfg(unix)]
fn same_device(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev(),
        _ => true,
    }
}

#[cfg(not(unix))]
fn same_device(_a: &Path, _b: &Path) -> bool {
    true
}

fn nearest_existing(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    while !current.exists() {
        if !current.pop() {
            break;
        }
    }
    current
}

fn apply(config: &Config, plan: &Plan) -> Result<()> {
    let vault = &config.vault_path;
    let from = vault.join(&plan.source);
    let to = vault.join(&plan.destination);

    let created = match to.parent() {
        Some(parent) => create_dirs(parent)?,
        None => Vec::new(),
    };

    if let Err(err) = std::fs::rename(&from, &to) {
        let mut message = format!(
            "Could not move {} to {}: {err}",
            plan.source, plan.destination
        );
        let leftovers = remove_dirs(&created);
        if !leftovers.is_empty() {
            message.push_str(&format!(
                "\nEmpty directories it had created could not be removed:\n  - {}",
                leftovers.join("\n  - ")
            ));
        }
        return Err(anyhow!("{message}"));
    }

    let mut written: Vec<(PathBuf, String)> = Vec::new();
    for edit in &plan.edits {
        let path = vault.join(&edit.new_path);
        if let Err(err) = write_atomic(&path, &edit.updated) {
            let failures = roll_back(&written, &to, &from, &created);
            if failures.is_empty() {
                return Err(anyhow!(
                    "Could not write {}: {err}\nNothing was changed: the move was rolled back.",
                    edit.new_path
                ));
            }
            return Err(anyhow!(
                "Could not write {}: {err}\n\
                 The rollback was incomplete, so the vault is in a mixed state:\n  - {}\n\
                 Put these right by hand before running knapper here again.",
                edit.new_path,
                failures.join("\n  - ")
            ));
        }
        written.push((path, edit.original.clone()));
    }

    Ok(())
}

// -------------------------------------------------------------- reporting --

/// What the report is describing: a plan that was carried out, a plan asked
/// for by `--dry-run`, or a plan refused because it would break org links.
/// The last two look the same on disk -- nothing happened -- and a report
/// that said "Done!" after refusing would be a lie about the vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Applied,
    Preview,
    Refused,
}

/// `blocked` is a fact about the plan, not about this run: a dry run of a
/// move that would be refused has to say so, or previewing it is useless.
fn report(plan: &Plan, outcome: Outcome, blocked: bool, format: &str) {
    let applied = outcome == Outcome::Applied;
    let files_updated: Vec<&String> = plan.edits.iter().map(|e| &e.new_path).collect();
    let unsupported: Vec<Value> = plan
        .unsupported
        .iter()
        .map(|u| json!({"file": u.file, "link": u.link, "reason": u.reason}))
        .collect();

    if format == "json" {
        let mut out = serde_json::Map::new();
        out.insert("kind".into(), json!("directory"));
        out.insert("old_path".into(), json!(plan.source));
        out.insert("new_path".into(), json!(plan.destination));
        out.insert("dry_run".into(), json!(outcome == Outcome::Preview));
        out.insert("applied".into(), json!(applied));
        out.insert("blocked".into(), json!(blocked));
        out.insert("entries".into(), json!(plan.entries.len()));
        out.insert("notes".into(), json!(plan.notes));
        out.insert("files_updated".into(), json!(files_updated));
        out.insert("links_updated".into(), json!(plan.links_updated()));
        out.insert("unsupported_links".into(), json!(unsupported));
        out.insert("warnings".into(), json!(plan.warnings));
        // The full plan is what a caller acts on when nothing happened.
        if !applied {
            out.insert(
                "moves".into(),
                json!(plan
                    .entries
                    .iter()
                    .map(|e| json!({
                        "from": e,
                        "to": remap(e, &plan.source, &plan.destination),
                    }))
                    .collect::<Vec<_>>()),
            );
            out.insert(
                "edits".into(),
                json!(plan
                    .edits
                    .iter()
                    .map(|e| json!({
                        "file": e.path,
                        "new_file": e.new_path,
                        "links": e.changes.len(),
                        "changes": e.changes.iter().map(|c| json!({
                            "line": c.line, "before": c.before, "after": c.after,
                        })).collect::<Vec<_>>(),
                    }))
                    .collect::<Vec<_>>()),
            );
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&Value::Object(out)).unwrap()
        );
        return;
    }

    let lead = match outcome {
        Outcome::Preview => "[DRY RUN] ",
        _ => "",
    };
    println!(
        "{lead}Moving directory: {} -> {}",
        plan.source, plan.destination
    );
    println!(
        "  {} files ({} notes) move with it",
        plan.entries.len(),
        plan.notes
    );

    if !applied {
        println!(
            "  Would update {} links in {} files:",
            plan.links_updated(),
            plan.edits.len()
        );
        for edit in &plan.edits {
            println!("    {}", edit.new_path);
            for change in &edit.changes {
                println!(
                    "      {}: {} -> {}",
                    change.line, change.before, change.after
                );
            }
        }
    } else {
        for edit in &plan.edits {
            println!(
                "  Updated {} links in {}",
                edit.changes.len(),
                edit.new_path
            );
        }
    }

    for warning in &plan.warnings {
        println!("  ⚠️ {warning}");
    }
    for entry in &plan.unsupported {
        println!(
            "  ⚠️ {}: [[{}]] cannot be rewritten ({})",
            entry.file, entry.link, entry.reason
        );
    }

    match outcome {
        Outcome::Applied => println!(
            "\nDone! Moved {} -> {} and updated {} links in {} files.",
            plan.source,
            plan.destination,
            plan.links_updated(),
            plan.edits.len()
        ),
        Outcome::Preview if blocked => println!(
            "\nNothing was written. As it stands this move would be refused: \
             rerun with --allow-broken-org-links to do it anyway."
        ),
        Outcome::Preview => println!("\nNothing was written."),
        Outcome::Refused => println!("\nNothing was written: the move was refused."),
    }
}

// ------------------------------------------------------------ the command --

pub fn move_directory(
    config: &Config,
    source_arg: &str,
    destination_arg: &str,
    options: &Options,
    format: &str,
) -> Result<()> {
    let vault = &config.vault_path;

    let source = vault_relative(config, source_arg)?;
    if source.is_empty() {
        bail!("Refusing to move the vault root");
    }
    let from = vault.join(&source);
    let meta = std::fs::symlink_metadata(&from)
        .map_err(|err| anyhow!("Directory not found: {source_arg} ({err})"))?;
    if meta.file_type().is_symlink() {
        bail!("Source is a symlink: {source}\nknapper moves real directories only.");
    }
    if !meta.is_dir() {
        bail!("Not a directory: {source}");
    }
    if is_excluded(&source, &config.exclude) {
        bail!("Source is excluded by {CONFIG_FILENAME}: {source}");
    }
    ensure_inside_vault(config, &source, "The source")?;

    let parent = vault_relative(config, destination_arg)?;
    let name = source.rsplit('/').next().unwrap_or(&source).to_string();
    let destination = join_rel(&parent, &name);

    if destination == source {
        bail!("{source} is already there");
    }
    if inside(&destination, &source) || inside(&parent, &source) {
        bail!("Destination is inside the source directory: {destination}");
    }
    if is_excluded(&destination, &config.exclude) {
        bail!("Destination is excluded by {CONFIG_FILENAME}: {destination}");
    }
    // Before the collision check, because a destination reached through a
    // symlink would answer that question about a file somewhere else.
    ensure_inside_vault(config, &destination, "The destination")?;
    let to = vault.join(&destination);
    if to.symlink_metadata().is_ok() {
        bail!("Target already exists: {destination}\nknapper does not merge directories.");
    }
    if !same_device(&from, &nearest_existing(&to)) {
        bail!(
            "{source} and {destination} are on different filesystems.\n\
             A directory move has to be a rename; copy it yourself, then rerun with the copy."
        );
    }

    let plan = build_plan(config, &source, &destination)?;

    // Org files are read but not rewritten. Leaving an inbound org link
    // pointing at a path that no longer exists, and reporting the move as a
    // success, is the one outcome worth refusing outright.
    let blocked = !plan.unsupported.is_empty() && !options.allow_broken_org_links;

    if options.dry_run {
        report(&plan, Outcome::Preview, blocked, format);
        return Ok(());
    }
    if blocked {
        report(&plan, Outcome::Refused, blocked, format);
        bail!(
            "{} inbound org link(s) point into {source}, and knapper does not rewrite org links.\n\
             Fix them first, or rerun with --allow-broken-org-links to move anyway.",
            plan.unsupported.len()
        );
    }

    apply(config, &plan)?;
    report(&plan, Outcome::Applied, blocked, format);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_are_computed_from_the_referring_folder() {
        for (from, to, expected) in [
            ("", "a.md", "a.md"),
            ("", "x/a.md", "x/a.md"),
            ("Notes", "a.md", "../a.md"),
            ("Notes", "Notes/a.md", "a.md"),
            ("Notes/Deep", "Other/a.md", "../../Other/a.md"),
            ("a/b/c", "a/b/d/e.md", "../d/e.md"),
        ] {
            assert_eq!(relative_from(from, to), expected, "{from} -> {to}");
        }
    }

    #[test]
    fn a_path_that_climbs_out_of_the_vault_is_refused() {
        assert_eq!(normalize("a/../b"), Some("b".to_string()));
        assert_eq!(normalize("./a/./b"), Some("a/b".to_string()));
        assert_eq!(normalize(".."), None);
        assert_eq!(normalize("a/../.."), None);
    }

    #[test]
    fn remapping_only_touches_the_subtree() {
        assert_eq!(
            remap("Docs/a.md", "Docs", "Archive/Docs"),
            "Archive/Docs/a.md"
        );
        assert_eq!(remap("Docs", "Docs", "Archive/Docs"), "Archive/Docs");
        assert_eq!(remap("Docsy/a.md", "Docs", "Archive/Docs"), "Docsy/a.md");
        assert_eq!(remap("Other.md", "Docs", "Archive/Docs"), "Other.md");
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
}
