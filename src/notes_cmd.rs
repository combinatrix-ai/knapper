//! The remaining note-level commands: context, frontmatter, lint, daily,
//! rename, move and init.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{anyhow, Result};
use chrono::{Duration, Local, NaiveDate};
use regex::Regex;
use serde_json::{json, Value};

use crate::graph::build_link_graph;
use crate::note::{parse_note, split_frontmatter};
use crate::vault::{all_notes, relative_path, resolve_path, Config};

fn print_json(value: &Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^(#{1,6})\s+(.+)$").unwrap());

// [text](target) or [text](target "title"), but not an image.
static MD_LINK_SUB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\[([^\]]*)\]\(([^)\s]+)((?:\s+"[^"]*")?)\)"#).unwrap());

/// What `context` should leave out. Building the link graph to find
/// backlinks is the expensive part, so skipping it is a real saving on a
/// large vault.
#[derive(Debug, Default, Clone, Copy)]
pub struct ContextOptions {
    pub no_content: bool,
    pub no_backlinks: bool,
    pub no_tasks: bool,
    pub max_content: Option<usize>,
}

pub fn context(config: &Config, file: &str, format: &str, options: &ContextOptions) -> Result<()> {
    let path = resolve_path(&config.vault_path, file);
    if !path.exists() {
        return Err(anyhow!("File not found: {}", path.display()));
    }
    let content = std::fs::read_to_string(&path)?;
    let note = parse_note(&path, &content);
    let relative = relative_path(&config.vault_path, &path);

    let mut out = serde_json::Map::new();
    out.insert("path".into(), json!(relative));
    out.insert("title".into(), json!(note.title));
    if !options.no_content {
        let content = match options.max_content {
            // The marker matters: without it a reader cannot tell a
            // truncated note from one that simply ends there.
            Some(limit) if note.content.chars().count() > limit => {
                let head: String = note.content.chars().take(limit).collect();
                format!("{head}\n... (truncated)")
            }
            _ => note.content.clone(),
        };
        out.insert("content".into(), json!(content));
    }
    if !note.links.is_empty() {
        let mut links = note.links.clone();
        links.sort();
        links.dedup();
        out.insert("links".into(), json!(links));
    }

    if !options.no_backlinks {
        let graph = build_link_graph(config);
        if let Some(incoming) = graph.incoming.get(&relative) {
            if !incoming.is_empty() {
                out.insert(
                    "backlinks".into(),
                    json!(incoming.iter().collect::<Vec<_>>()),
                );
            }
        }
    }

    if !note.inline_fields.is_empty() {
        out.insert("inline_fields".into(), json!(note.inline_fields));
    }
    if !note.tags.is_empty() {
        let mut tags = note.tags.clone();
        tags.sort();
        out.insert("tags".into(), json!(tags));
    }

    if !options.no_tasks {
        let filters = crate::tasks::Filters {
            include_done: true,
            file: Some(&relative),
            exclude: &[],
            status: &[],
            ..Default::default()
        };
        if let Ok(found) = crate::tasks::find_tasks(config, &filters) {
            if !found.is_empty() {
                let items: Vec<Value> = found
                    .iter()
                    .map(|t| json!({"line": t.line, "text": t.text, "done": t.done}))
                    .collect();
                out.insert("tasks".into(), json!(items));
            }
        }
    }

    let headings: Vec<Value> = HEADING
        .captures_iter(&note.content)
        .map(|c| {
            let line = note.content[..c.get(0).unwrap().start()]
                .matches('\n')
                .count()
                + 1;
            json!({"level": c[1].len(), "text": c[2].trim(), "line": line})
        })
        .collect();
    if !headings.is_empty() {
        out.insert("headings".into(), json!(headings));
    }

    out.insert(
        "stats".into(),
        json!({
            "chars": note.content.chars().count(),
            "words": note.content.split_whitespace().count(),
            "lines": if note.content.is_empty() { 0 } else { note.content.matches('\n').count() + 1 },
        }),
    );

    if format == "json" {
        print_json(&Value::Object(out));
        return Ok(());
    }

    println!("# {}\n", out["path"].as_str().unwrap_or_default());
    if let Some(tags) = out.get("tags") {
        println!("## Tags");
        println!(
            "  {}",
            tags.as_array()
                .map(|a| a
                    .iter()
                    .map(|t| format!("#{}", t.as_str().unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default()
        );
    }
    Ok(())
}

pub fn frontmatter_get(config: &Config, file: &str, key: Option<&str>, format: &str) -> Result<()> {
    let path = resolve_path(&config.vault_path, file);
    let content = std::fs::read_to_string(&path)?;
    let (meta, _) = split_frontmatter(&content);

    match key {
        Some(key) => match meta.get(serde_yaml::Value::String(key.into())) {
            Some(value) => match value {
                serde_yaml::Value::String(s) => println!("{s}"),
                other => println!("{}", serde_yaml::to_string(other)?.trim()),
            },
            None => return Err(anyhow!("Key not found: {key}")),
        },
        None => {
            if format == "json" {
                print_json(&crate::query::yaml_mapping_to_json(&meta));
            } else {
                print!("{}", serde_yaml::to_string(&meta)?);
            }
        }
    }
    Ok(())
}

pub fn lint(config: &Config, checks: &[String], format: &str) -> Result<()> {
    let all: Vec<String> = [
        "broken-links",
        "orphans",
        "duplicates",
        "empty",
        "frontmatter",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let checks = if checks.is_empty() { &all } else { checks };

    let mut issues: Vec<Value> = Vec::new();
    let mut summary = serde_json::Map::new();
    let mut total = 0usize;

    if format == "text" {
        println!("Running lint checks...\n");
    }

    // The link graph reads every note, so build it at most once even though
    // two checks need it.
    let graph = checks
        .iter()
        .any(|c| c == "broken-links" || c == "orphans")
        .then(|| build_link_graph(config));

    if checks.iter().any(|c| c == "broken-links") {
        let graph = graph.as_ref().unwrap();
        let count: usize = graph.broken.values().map(|v| v.len()).sum();
        summary.insert("broken_links".into(), json!(count));
        total += count;
        for (file, links) in &graph.broken {
            for link in links {
                issues.push(json!({
                    "type": "broken-link", "file": file,
                    "detail": format!("[[{link}]]"), "severity": "warning"
                }));
            }
        }
        if format == "text" {
            if count > 0 {
                println!("❌ Broken links: {count}");
            } else {
                println!("✅ Broken links: 0");
            }
        }
    }

    if checks.iter().any(|c| c == "orphans") {
        let graph = graph.as_ref().unwrap();
        let orphans: Vec<&String> = graph
            .files
            .iter()
            .filter(|f| !f.starts_with("Templates/") && !f.starts_with('.'))
            .filter(|f| graph.incoming.get(*f).map_or(true, |i| i.is_empty()))
            .collect();
        summary.insert("orphans".into(), json!(orphans.len()));
        total += orphans.len();
        for file in &orphans {
            issues.push(json!({
                "type": "orphan", "file": file,
                "detail": "No incoming links", "severity": "info"
            }));
        }
        if format == "text" {
            let mark = if orphans.len() > 10 {
                "⚠️"
            } else if orphans.is_empty() {
                "✅"
            } else {
                "ℹ️"
            };
            println!("{mark} Orphan notes: {}", orphans.len());
        }
    }

    let notes = all_notes(config);

    if checks.iter().any(|c| c == "duplicates") {
        let mut by_stem: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for path in &notes {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            by_stem
                .entry(stem)
                .or_default()
                .push(relative_path(&config.vault_path, path));
        }
        let dups: Vec<_> = by_stem.iter().filter(|(_, v)| v.len() > 1).collect();
        summary.insert("duplicates".into(), json!(dups.len()));
        total += dups.len();
        for (name, paths) in &dups {
            issues.push(json!({
                "type": "duplicate", "file": paths[0],
                "detail": format!("Also at: {}", paths[1..].join(", ")),
                "severity": "warning", "name": name
            }));
        }
        if format == "text" {
            if dups.is_empty() {
                println!("✅ Duplicate names: 0");
            } else {
                println!("⚠️ Duplicate names: {}", dups.len());
            }
        }
    }

    if checks.iter().any(|c| c == "empty") {
        let mut empty = Vec::new();
        for path in &notes {
            let relative = relative_path(&config.vault_path, path);
            if relative.starts_with("Templates/") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let (_, body) = split_frontmatter(&content);
            if body.trim().chars().count() < 10 {
                empty.push(relative);
            }
        }
        empty.sort();
        summary.insert("empty".into(), json!(empty.len()));
        total += empty.len();
        for file in &empty {
            issues.push(json!({
                "type": "empty", "file": file,
                "detail": "Very little content", "severity": "info"
            }));
        }
        if format == "text" {
            if empty.is_empty() {
                println!("✅ Empty notes: 0");
            } else {
                println!("ℹ️ Empty notes: {}", empty.len());
            }
        }
    }

    if checks.iter().any(|c| c == "frontmatter") {
        let mut missing = Vec::new();
        for path in &notes {
            let relative = relative_path(&config.vault_path, path);
            if relative.starts_with("Templates/") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            if !content.starts_with("---") {
                missing.push(relative);
            }
        }
        missing.sort();
        summary.insert("missing_frontmatter".into(), json!(missing.len()));
        for file in &missing {
            issues.push(json!({
                "type": "frontmatter", "file": file,
                "detail": "No frontmatter", "severity": "info"
            }));
        }
        if format == "text" {
            println!("ℹ️ Missing frontmatter: {}", missing.len());
        }
    }

    summary.insert("total_issues".into(), json!(total));

    if format == "json" {
        print_json(&json!({"issues": issues, "summary": summary}));
    } else {
        println!("\nTotal issues: {total}");
    }
    Ok(())
}

/// Moment-style tokens, which is what both Templater and Core Templates use.
fn format_date(format: &str, date: NaiveDate) -> String {
    format
        .replace("YYYY", &date.format("%Y").to_string())
        .replace("MM", &date.format("%m").to_string())
        .replace("DD", &date.format("%d").to_string())
}

fn parse_relative_date(spec: &str) -> Option<NaiveDate> {
    let today = Local::now().date_naive();
    match spec.to_lowercase().as_str() {
        "" | "today" => Some(today),
        "yesterday" => Some(today - Duration::days(1)),
        "tomorrow" => Some(today + Duration::days(1)),
        other => NaiveDate::parse_from_str(other, "%Y-%m-%d").ok(),
    }
}

pub fn daily(config: &Config, date: Option<&str>, path_only: bool, format: &str) -> Result<()> {
    let date = parse_relative_date(date.unwrap_or("today"))
        .ok_or_else(|| anyhow!("Invalid date: {}", date.unwrap_or("")))?;
    let name = format_date(&config.daily_format, date);
    let relative = format!("{}/{name}.md", config.daily_folder.trim_end_matches('/'));
    let path = config.vault_path.join(&relative);

    let created = !path.exists();
    if created {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let template = config.vault_path.join(&config.daily_template);
        let body = match std::fs::read_to_string(&template) {
            Ok(content) => crate::templater::expand(&content, &config.template_engine, &name, date),
            Err(_) => format!("# {name}\n"),
        };
        std::fs::write(&path, body)?;
    }

    if path_only {
        println!("{relative}");
    } else if format == "json" {
        print_json(&json!({"path": relative, "date": name, "created": created}));
    } else if created {
        println!("Created: {relative}");
    } else {
        println!("{relative}");
    }
    Ok(())
}

fn split_markdown_target(href: &str) -> (String, String, bool) {
    let encoded = href.contains("%20");
    let decoded = percent_encoding::percent_decode_str(href)
        .decode_utf8_lossy()
        .to_string();
    let mut target = decoded;
    let mut anchor = String::new();
    for sep in ['#', '^'] {
        if let Some(idx) = target.find(sep) {
            if idx > 0 {
                anchor = target[idx..].to_string();
                target = target[..idx].to_string();
                break;
            }
        }
    }
    if target.to_lowercase().ends_with(".md") {
        target.truncate(target.len() - 3);
    }
    (target, anchor, encoded)
}

fn markdown_link_targets_note(href: &str, stem: &str) -> bool {
    static EXTERNAL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[a-zA-Z][a-zA-Z0-9+.\-]*:").unwrap());
    if EXTERNAL.is_match(href) || href.starts_with('#') || href.starts_with("//") {
        return false;
    }
    let (target, _, _) = split_markdown_target(href);
    !target.is_empty()
        && Path::new(&target)
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase() == stem.to_lowercase())
            .unwrap_or(false)
}

fn update_links_in_file(
    path: &Path,
    old_stem: &str,
    new_stem: &str,
    old_folder: Option<&str>,
    new_folder: Option<&str>,
) -> Result<usize> {
    let original = std::fs::read_to_string(path)?;

    // A wikilink may carry a #heading or ^block-id before its |alias.
    // Missing that group left [[Note#Heading]] untouched by a rename, which
    // turned it into a broken link.
    const ANCHOR: &str = r"((?:#|\^)[^\]|]*)?";
    let wiki = match old_folder {
        Some(folder) => Regex::new(&format!(
            r"(?i)\[\[({}/)?{}{}(\|[^\]]+)?\]\]",
            regex::escape(folder),
            regex::escape(old_stem),
            ANCHOR
        ))?,
        None => Regex::new(&format!(
            r"(?i)\[\[([^\]|#^]*[/\\])?{}{}(\|[^\]]+)?\]\]",
            regex::escape(old_stem),
            ANCHOR
        ))?,
    };

    let wiki_count = wiki.find_iter(&original).count();
    let content = wiki.replace_all(&original, |c: &regex::Captures| {
        let prefix = c.get(1).map(|m| m.as_str()).unwrap_or("");
        let anchor = c.get(2).map(|m| m.as_str()).unwrap_or("");
        let alias = c.get(3).map(|m| m.as_str()).unwrap_or("");
        match (new_folder, old_folder) {
            (Some(new), old) if Some(new) != old => {
                format!("[[{new}/{new_stem}{anchor}{alias}]]")
            }
            _ if !prefix.is_empty() => format!("[[{prefix}{new_stem}{anchor}{alias}]]"),
            _ => format!("[[{new_stem}{anchor}{alias}]]"),
        }
    });

    let mut markdown_count = 0;
    let content = MD_LINK_SUB.replace_all(&content, |c: &regex::Captures| {
        let whole = c.get(0).unwrap();
        let text = &c[1];
        let href = &c[2];
        let title = &c[3];

        // Images are not note links; the regex crate has no lookbehind.
        let is_image =
            whole.start() > 0 && content.as_bytes().get(whole.start() - 1) == Some(&b'!');
        if is_image || !markdown_link_targets_note(href, old_stem) {
            return whole.as_str().to_string();
        }

        let (target, anchor, encoded) = split_markdown_target(href);
        let folder = Path::new(&target)
            .parent()
            .filter(|p| !p.as_os_str().is_empty() && *p != Path::new("."))
            .map(|p| p.to_string_lossy().into_owned());
        let folder = match (new_folder, old_folder) {
            (Some(new), old) if Some(new) != old => Some(new.to_string()),
            _ => folder,
        };

        let new_target = match folder {
            Some(f) => format!("{f}/{new_stem}"),
            None => new_stem.to_string(),
        };
        let mut new_href = format!("{new_target}.md{anchor}");
        if encoded || new_href.contains(' ') {
            new_href = new_href.replace(' ', "%20");
        }
        markdown_count += 1;
        format!("[{text}]({new_href}{title})")
    });

    if content != original {
        std::fs::write(path, content.as_ref())?;
        return Ok(wiki_count + markdown_count);
    }
    Ok(0)
}

pub fn rename(config: &Config, old: &str, new: &str, dry_run: bool, format: &str) -> Result<()> {
    let old_stem = old.trim_end_matches(".md");
    let new_stem = new.trim_end_matches(".md");

    let old_path = all_notes(config)
        .into_iter()
        .find(|p| {
            p.file_stem()
                .map(|s| s.to_string_lossy() == old_stem)
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow!("Note not found: {old}"))?;

    let old_relative = relative_path(&config.vault_path, &old_path);
    let old_folder = old_path
        .parent()
        .and_then(|p| p.strip_prefix(&config.vault_path).ok())
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty());

    let new_path = old_path.with_file_name(format!("{new_stem}.md"));
    let new_relative = relative_path(&config.vault_path, &new_path);

    if new_path.exists() && new_path != old_path {
        return Err(anyhow!("Target already exists: {new_relative}"));
    }

    // Which files hold a link to it. The rewrite itself decides what changes.
    let linking: Vec<PathBuf> = all_notes(config)
        .into_iter()
        .filter(|p| {
            std::fs::read_to_string(p)
                .map(|c| c.to_lowercase().contains(&old_stem.to_lowercase()))
                .unwrap_or(false)
        })
        .collect();

    if format == "text" {
        println!("Renaming: {old_relative} -> {new_relative}");
        println!("Found {} files with links to update", linking.len());
    }

    if dry_run {
        if format == "text" {
            println!("\n[DRY RUN] Would update:");
            for path in &linking {
                println!("  {}", relative_path(&config.vault_path, path));
            }
        } else {
            print_json(&json!({
                "old_path": old_relative, "new_path": new_relative, "dry_run": true,
                "files_to_update": linking.iter()
                    .map(|p| relative_path(&config.vault_path, p)).collect::<Vec<_>>()
            }));
        }
        return Ok(());
    }

    std::fs::rename(&old_path, &new_path)?;

    let mut updated_files = Vec::new();
    let mut updated_links = 0;
    for path in &linking {
        let path = if *path == old_path { &new_path } else { path };
        let count = update_links_in_file(
            path,
            old_stem,
            new_stem,
            old_folder.as_deref(),
            old_folder.as_deref(),
        )?;
        if count > 0 {
            updated_links += count;
            updated_files.push(relative_path(&config.vault_path, path));
            if format == "text" {
                println!(
                    "  Updated {count} links in {}",
                    relative_path(&config.vault_path, path)
                );
            }
        }
    }

    if format == "json" {
        print_json(&json!({
            "old_path": old_relative, "new_path": new_relative,
            "files_updated": updated_files, "links_updated": updated_links
        }));
    } else {
        println!(
            "\nDone! Renamed and updated {updated_links} links in {} files.",
            updated_files.len()
        );
    }
    Ok(())
}

pub const DEFAULT_CONFIG: &str = include_str!("default_config.md");

pub fn init(force: bool) -> Result<()> {
    let path = std::env::current_dir()?.join(crate::vault::CONFIG_FILENAME);
    if path.exists() && !force {
        return Err(anyhow!(
            "Config file already exists: {}\nUse --force to overwrite.",
            path.display()
        ));
    }
    std::fs::write(&path, DEFAULT_CONFIG)?;
    println!("Created: {}", path.display());
    Ok(())
}

pub fn move_note(
    config: &Config,
    source: &str,
    destination: &str,
    dry_run: bool,
    format: &str,
) -> Result<()> {
    let stem = Path::new(source)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow!("Invalid source: {source}"))?;

    let old_path = all_notes(config)
        .into_iter()
        .find(|p| {
            relative_path(&config.vault_path, p) == source
                || p.file_stem()
                    .map(|s| s.to_string_lossy() == stem)
                    .unwrap_or(false)
        })
        .ok_or_else(|| anyhow!("Note not found: {source}"))?;

    let old_relative = relative_path(&config.vault_path, &old_path);
    let old_folder = Path::new(&old_relative)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty());

    // A destination that names a directory keeps the filename.
    let destination_path = config.vault_path.join(destination);
    let new_path = if destination.ends_with('/') || destination_path.is_dir() {
        destination_path.join(format!("{stem}.md"))
    } else if destination.ends_with(".md") {
        destination_path
    } else {
        destination_path.join(format!("{stem}.md"))
    };
    let new_relative = relative_path(&config.vault_path, &new_path);
    let new_folder = Path::new(&new_relative)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty());

    if new_path.exists() && new_path != old_path {
        return Err(anyhow!("Target already exists: {new_relative}"));
    }

    let linking: Vec<PathBuf> = all_notes(config)
        .into_iter()
        .filter(|p| {
            std::fs::read_to_string(p)
                .map(|c| c.to_lowercase().contains(&stem.to_lowercase()))
                .unwrap_or(false)
        })
        .collect();

    if format == "text" {
        println!("Moving: {old_relative} -> {new_relative}");
        println!("Found {} files with links to update", linking.len());
    }

    if dry_run {
        if format == "text" {
            println!("\n[DRY RUN] Would update:");
            for path in &linking {
                println!("  {}", relative_path(&config.vault_path, path));
            }
        } else {
            print_json(&json!({
                "old_path": old_relative, "new_path": new_relative, "dry_run": true,
                "files_to_update": linking.iter()
                    .map(|p| relative_path(&config.vault_path, p)).collect::<Vec<_>>()
            }));
        }
        return Ok(());
    }

    if let Some(parent) = new_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&old_path, &new_path)?;

    let mut updated_files = Vec::new();
    let mut updated_links = 0;
    for path in &linking {
        let path = if *path == old_path { &new_path } else { path };
        let count = update_links_in_file(
            path,
            &stem,
            &stem,
            old_folder.as_deref(),
            new_folder.as_deref(),
        )?;
        if count > 0 {
            updated_links += count;
            updated_files.push(relative_path(&config.vault_path, path));
            if format == "text" {
                println!(
                    "  Updated {count} links in {}",
                    relative_path(&config.vault_path, path)
                );
            }
        }
    }

    if format == "json" {
        print_json(&json!({
            "old_path": old_relative, "new_path": new_relative,
            "files_updated": updated_files, "links_updated": updated_links
        }));
    } else {
        println!(
            "\nDone! Moved and updated {updated_links} links in {} files.",
            updated_files.len()
        );
    }
    Ok(())
}

/// Rewrite one frontmatter key, leaving the body byte-for-byte alone.
fn rewrite_frontmatter(
    path: &Path,
    edit: impl FnOnce(&mut serde_yaml::Mapping) -> Result<()>,
) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let (mut meta, body) = split_frontmatter(&content);
    let body = body.to_string();

    edit(&mut meta)?;

    let header = if meta.is_empty() {
        String::new()
    } else {
        format!("---\n{}---\n\n", serde_yaml::to_string(&meta)?)
    };
    std::fs::write(path, format!("{header}{body}"))?;
    Ok(())
}

/// Parse a value the way YAML would, so numbers and booleans stay typed.
fn scalar(value: &str) -> serde_yaml::Value {
    serde_yaml::from_str(value).unwrap_or_else(|_| serde_yaml::Value::String(value.to_string()))
}

pub fn frontmatter_set(config: &Config, file: &str, key: &str, value: &str) -> Result<()> {
    let path = resolve_path(&config.vault_path, file);
    if !path.exists() {
        return Err(anyhow!("File not found: {}", path.display()));
    }
    rewrite_frontmatter(&path, |meta| {
        meta.insert(serde_yaml::Value::String(key.into()), scalar(value));
        Ok(())
    })?;
    println!("Set {key} = {value}");
    Ok(())
}

pub fn frontmatter_delete(config: &Config, file: &str, key: &str) -> Result<()> {
    let path = resolve_path(&config.vault_path, file);
    if !path.exists() {
        return Err(anyhow!("File not found: {}", path.display()));
    }
    let mut existed = false;
    rewrite_frontmatter(&path, |meta| {
        existed = meta.remove(serde_yaml::Value::String(key.into())).is_some();
        Ok(())
    })?;
    if !existed {
        return Err(anyhow!("Key not found: {key}"));
    }
    println!("Deleted {key}");
    Ok(())
}
