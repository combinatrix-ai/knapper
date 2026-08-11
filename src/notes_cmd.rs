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
static ATX_HEADING_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})[ \t]+(.+?)[ \t]*$").expect("valid ATX heading regex"));

// [text](target "title"), with the target optionally wrapped in <>, which is
// how a path holding spaces is written without encoding them. Groups: text,
// angle-wrapped target, bare target, title.
static MD_LINK_SUB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\[([^\]]*)\]\(\s*(?:<([^>]*)>|([^)\s]+))((?:\s+"[^"]*")?)\s*\)"#).unwrap()
});

/// What `context` should leave out. Building the link graph to find
/// backlinks is the expensive part, so skipping it is a real saving on a
/// large vault.
#[derive(Debug, Default, Clone, Copy)]
pub struct ContextOptions {
    pub no_content: bool,
    pub no_backlinks: bool,
    pub no_tasks: bool,
    pub max_content: Option<usize>,
    /// Physical, 1-based focused line. `None` preserves the original context
    /// contract and output shape.
    pub line: Option<usize>,
    /// Explicit physical context window. Focused mode defaults both to 3.
    pub before: Option<usize>,
    pub after: Option<usize>,
    pub section: bool,
    pub outline_depth: Option<usize>,
}

pub fn context(config: &Config, file: &str, format: &str, options: &ContextOptions) -> Result<()> {
    let path = resolve_path(&config.vault_path, file);
    if !path.exists() {
        return Err(anyhow!("File not found: {}", path.display()));
    }
    let content = std::fs::read_to_string(&path)?;
    if options.line.is_some()
        || options.before.is_some()
        || options.after.is_some()
        || options.section
        || options.outline_depth.is_some()
    {
        return focused_context(config, &path, format, options, &content);
    }
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

    // External references are not links -- they point outside the vault, and
    // nothing here resolves them -- so they would otherwise be invisible to a
    // caller that has only this one aggregated view.
    let references = crate::refs::parse_refs(&relative, &content);
    if !references.is_empty() {
        let items: Vec<Value> = references
            .iter()
            .map(|reference| {
                json!({
                    "uri": &reference.uri,
                    "provider": &reference.provider,
                    "locator": &reference.locator,
                    "label": &reference.label,
                    "line": reference.line,
                    "column": reference.column,
                })
            })
            .collect();
        out.insert("references".into(), json!(items));
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

#[derive(Debug, Clone)]
struct FocusHeading {
    level: usize,
    text: String,
    line: usize,
    end_line: usize,
}

#[derive(Debug, Clone)]
struct OutlineEntry {
    heading_index: usize,
    top_index: usize,
    depth: usize,
}

#[derive(Debug, Clone)]
struct FocusExcerpt {
    start_line: usize,
    end_line: usize,
    truncated: bool,
    content: String,
}

/// Split physical source lines using the same convention as `rg`, editors and
/// the rest of knapper: a final newline terminates the preceding line; it does
/// not invent an additional addressable empty line.
fn physical_lines(content: &str) -> Vec<&str> {
    content.lines().collect()
}

fn display_line(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

fn markdown_headings(content: &str, total_lines: usize) -> Vec<FocusHeading> {
    let (_, body) = split_frontmatter(content);
    let prefix_len = content.len().saturating_sub(body.len());
    let prefix_lines = content[..prefix_len].matches('\n').count();
    let masked = crate::parser::mask_noncontent(body);
    let mut headings = Vec::new();

    for (index, (original, masked_line)) in body.split('\n').zip(masked.split('\n')).enumerate() {
        let masked_line = display_line(masked_line);
        let Some(masked_capture) = ATX_HEADING_LINE.captures(masked_line) else {
            continue;
        };
        // The masked line proves this is not a heading inside a code fence or
        // comment. Read the title from the original line so inline code and
        // Unicode remain exactly as authored.
        let original = display_line(original);
        let Some(capture) = ATX_HEADING_LINE.captures(original) else {
            continue;
        };
        let level = masked_capture[1].len();
        headings.push(FocusHeading {
            level,
            text: capture[2].trim().to_string(),
            line: prefix_lines + index + 1,
            end_line: total_lines,
        });
    }

    // A section ends immediately before the next heading at the same or a
    // shallower level. Nested headings remain inside their parent's range.
    for index in 0..headings.len() {
        if let Some(next) = headings[index + 1..]
            .iter()
            .find(|heading| heading.level <= headings[index].level)
        {
            headings[index].end_line = next.line.saturating_sub(1);
        }
    }
    headings
}

fn heading_parents(headings: &[FocusHeading]) -> Vec<Option<usize>> {
    let mut parents = vec![None; headings.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (index, heading) in headings.iter().enumerate() {
        while stack
            .last()
            .is_some_and(|parent| headings[*parent].level >= heading.level)
        {
            stack.pop();
        }
        parents[index] = stack.last().copied();
        stack.push(index);
    }
    parents
}

fn top_level_headings(
    headings: &[FocusHeading],
    parents: &[Option<usize>],
) -> (Option<String>, Vec<usize>) {
    let h1: Vec<usize> = headings
        .iter()
        .enumerate()
        .filter_map(|(index, heading)| (heading.level == 1).then_some(index))
        .collect();
    let title = h1.first().map(|index| headings[*index].text.clone());

    if h1.len() == 1 {
        let title_index = h1[0];
        let children: Vec<usize> = headings
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                parents[index]
                    .filter(|parent| *parent == title_index)
                    .map(|_| index)
            })
            .collect();
        if !children.is_empty() {
            return (title, children);
        }
        // A lone H1 with no children is the document title, not a duplicate
        // outline entry.
        return (title, Vec::new());
    }
    if h1.len() > 1 {
        // Multiple H1s are peer roots, so no one heading can be called the
        // document title. Keep the roots themselves as the first layer.
        return (None, h1);
    }

    let shallowest = headings.iter().map(|heading| heading.level).min();
    let top = shallowest
        .map(|level| {
            headings
                .iter()
                .enumerate()
                .filter_map(|(index, heading)| (heading.level == level).then_some(index))
                .collect()
        })
        .unwrap_or_default();
    (None, top)
}

fn top_ancestor(mut index: usize, parents: &[Option<usize>], tops: &[usize]) -> Option<usize> {
    loop {
        if tops.contains(&index) {
            return Some(index);
        }
        index = parents[index]?;
    }
}

fn heading_path(line: usize, headings: &[FocusHeading], parents: &[Option<usize>]) -> Vec<String> {
    let Some(mut index) = headings
        .iter()
        .enumerate()
        .rev()
        .find(|(_, heading)| heading.line <= line)
        .map(|(index, _)| index)
    else {
        return Vec::new();
    };
    // The nearest preceding heading may be a sibling that has already ended.
    while headings[index].end_line < line {
        let Some(parent) = parents[index] else {
            return Vec::new();
        };
        index = parent;
    }
    let mut path = Vec::new();
    loop {
        path.push(headings[index].text.clone());
        let Some(parent) = parents[index] else {
            break;
        };
        index = parent;
    }
    path.reverse();
    path
}

fn section_for_line(line: usize, headings: &[FocusHeading]) -> Option<(usize, usize)> {
    headings
        .iter()
        .filter(|heading| heading.line <= line && line <= heading.end_line)
        .max_by_key(|heading| (heading.level, heading.line))
        .map(|heading| (heading.line, heading.end_line))
}

fn outline_entries(
    headings: &[FocusHeading],
    parents: &[Option<usize>],
    tops: &[usize],
    depth_limit: usize,
) -> Vec<OutlineEntry> {
    if depth_limit == 0 {
        return Vec::new();
    }
    headings
        .iter()
        .enumerate()
        .filter_map(|(index, _)| {
            let top = top_ancestor(index, parents, tops)?;
            let mut depth = 1;
            let mut cursor = index;
            while cursor != top {
                cursor = parents[cursor]?;
                depth += 1;
            }
            (depth <= depth_limit).then_some(OutlineEntry {
                heading_index: index,
                top_index: top,
                depth,
            })
        })
        .collect()
}

fn selected_outline_indices(total: usize, current: Option<usize>) -> Vec<usize> {
    if total <= 12 {
        return (0..total).collect();
    }
    let mut selected = std::collections::BTreeSet::new();
    selected.extend(0..3.min(total));
    selected.extend(total.saturating_sub(3)..total);
    if let Some(current) = current {
        selected.extend(current.saturating_sub(2)..=(current + 2).min(total - 1));
    }
    selected.into_iter().collect()
}

fn focused_excerpt(
    lines: &[&str],
    start_line: usize,
    end_line: usize,
    focus_line: usize,
    max_content: Option<usize>,
) -> FocusExcerpt {
    let start = start_line.max(1).min(lines.len());
    let end = end_line.max(start).min(lines.len());
    let raw: Vec<&str> = lines[start - 1..end]
        .iter()
        .map(|line| display_line(line))
        .collect();
    let full = raw.join("\n");
    let Some(limit) = max_content.filter(|_| full.chars().count() > max_content.unwrap_or(0))
    else {
        return FocusExcerpt {
            start_line: start,
            end_line: end,
            truncated: false,
            content: full,
        };
    };
    let focus_index = focus_line
        .saturating_sub(start)
        .min(raw.len().saturating_sub(1));
    let focus_text = raw.get(focus_index).copied().unwrap_or_default();
    if focus_text.chars().count() >= limit {
        return FocusExcerpt {
            start_line: focus_line,
            end_line: focus_line,
            truncated: true,
            content: focus_text.chars().take(limit).collect::<String>(),
        };
    }

    // Grow outwards from the focused line, preserving the focus even when a
    // very small character budget cannot fit the complete requested window.
    // Alternate sides so a long line on one side cannot starve the other.
    let mut chosen_start = focus_index;
    let mut chosen_end = focus_index + 1;
    let mut used = focus_text.chars().count();
    let mut prefer_before = true;
    loop {
        let before = chosen_start.checked_sub(1);
        let after = (chosen_end < raw.len()).then_some(chosen_end);
        let candidates = if prefer_before {
            [before, after]
        } else {
            [after, before]
        };
        let mut added = false;
        for index in candidates.into_iter().flatten() {
            let cost = raw[index].chars().count() + 1; // the joining newline
            if used + cost > limit {
                continue;
            }
            used += cost;
            if index < chosen_start {
                chosen_start = index;
                prefer_before = false;
            } else {
                chosen_end = index + 1;
                prefer_before = true;
            }
            added = true;
            break;
        }
        if !added {
            break;
        }
    }
    FocusExcerpt {
        start_line: start + chosen_start,
        end_line: start + chosen_end - 1,
        truncated: true,
        content: raw[chosen_start..chosen_end].join("\n"),
    }
}

fn focused_context(
    config: &Config,
    path: &Path,
    format: &str,
    options: &ContextOptions,
    content: &str,
) -> Result<()> {
    if path.extension().and_then(|extension| extension.to_str()) == Some("org") {
        return Err(anyhow!(
            "Focused context (--line) currently supports Markdown notes only; org-mode notes do not have a Markdown outline"
        ));
    }
    if options.line.is_none() {
        return Err(anyhow!(
            "--before, --after, --section and --outline-depth require --line N"
        ));
    }
    if options.section && (options.before.is_some() || options.after.is_some()) {
        return Err(anyhow!(
            "--section cannot be combined with explicit --before or --after"
        ));
    }

    let lines = physical_lines(content);
    let line = options.line.unwrap();
    if line == 0 || line > lines.len() {
        return Err(anyhow!(
            "Invalid --line {line}: expected a physical line between 1 and {}",
            lines.len()
        ));
    }
    let headings = markdown_headings(content, lines.len());
    let parents = heading_parents(&headings);
    let (document_title, tops) = top_level_headings(&headings, &parents);
    let depth_limit = options.outline_depth.unwrap_or(1);
    let entries = outline_entries(&headings, &parents, &tops, depth_limit);
    let current_top = tops
        .iter()
        .enumerate()
        .find(|(_, index)| headings[**index].line <= line && line <= headings[**index].end_line)
        .map(|(index, _)| index);
    let current_outline = current_top.and_then(|top| {
        entries
            .iter()
            .position(|entry| entry.top_index == tops[top] && entry.depth == 1)
    });
    let selected = selected_outline_indices(entries.len(), current_outline);

    let section = section_for_line(line, &headings);
    let (excerpt_start, excerpt_end) = if options.section {
        section.unwrap_or((line, line))
    } else {
        (
            line.saturating_sub(options.before.unwrap_or(3)).max(1),
            line.saturating_add(options.after.unwrap_or(3))
                .min(lines.len()),
        )
    };
    let excerpt = focused_excerpt(
        &lines,
        excerpt_start,
        excerpt_end,
        line,
        options.max_content.filter(|_| !options.no_content),
    );
    let breadcrumb = heading_path(line, &headings, &parents);
    let relative = relative_path(&config.vault_path, path);

    if format == "json" {
        let outline_entries: Vec<Value> = selected
            .iter()
            .map(|selected_index| {
                let entry = &entries[*selected_index];
                let heading = &headings[entry.heading_index];
                json!({
                    "index": selected_index,
                    "title": heading.text,
                    "level": heading.level,
                    "depth": entry.depth,
                    "start_line": heading.line,
                    "end_line": heading.end_line,
                    "current": entry.depth == 1 && current_outline == Some(*selected_index),
                })
            })
            .collect();
        let omitted = entries.len().saturating_sub(selected.len());
        let mut focus = serde_json::Map::new();
        focus.insert("line".into(), json!(line));
        focus.insert("heading_path".into(), json!(breadcrumb));
        focus.insert(
            "enclosing_section".into(),
            section.map_or(
                Value::Null,
                |(start, end)| json!({"start_line": start, "end_line": end}),
            ),
        );

        let mut excerpt_json = serde_json::Map::new();
        excerpt_json.insert("start_line".into(), json!(excerpt.start_line));
        excerpt_json.insert("end_line".into(), json!(excerpt.end_line));
        excerpt_json.insert("truncated".into(), json!(excerpt.truncated));
        if !options.no_content {
            excerpt_json.insert("content".into(), json!(excerpt.content));
        }

        let mut out = serde_json::Map::new();
        out.insert("kind".into(), json!("note"));
        out.insert("path".into(), json!(relative));
        out.insert(
            "document".into(),
            json!({
                "title": document_title,
                "outline_depth": depth_limit,
                "entries": outline_entries,
                "total": entries.len(),
                "omitted": omitted,
                "truncated": omitted > 0,
                "current_branch_index": current_outline,
            }),
        );
        out.insert("focus".into(), Value::Object(focus));
        out.insert("excerpt".into(), Value::Object(excerpt_json));
        out.insert(
            "stats".into(),
            json!({
                "chars": content.chars().count(),
                "words": content.split_whitespace().count(),
                "lines": lines.len(),
            }),
        );
        print_json(&Value::Object(out));
        return Ok(());
    }

    println!("# {relative}\n");
    println!("Document");
    if let Some(title) = document_title {
        println!("  {title}");
    }
    if depth_limit == 0 {
        println!("  (outline hidden; use --outline-depth N)");
    } else if entries.is_empty() {
        println!("  (no headings)");
    } else {
        let mut previous = None;
        for selected_index in &selected {
            if let Some(previous) = previous {
                if *selected_index > previous + 1 {
                    println!("  … {} entries omitted", selected_index - previous - 1);
                }
            }
            let entry = &entries[*selected_index];
            let heading = &headings[entry.heading_index];
            let marker = if current_top
                .and_then(|top| tops.get(top).copied())
                .is_some_and(|top| top == entry.top_index && entry.depth == 1)
            {
                "▸"
            } else {
                " "
            };
            println!(
                "  {marker} {}{}  lines {}–{}",
                "  ".repeat(entry.depth.saturating_sub(1)),
                heading.text,
                heading.line,
                heading.end_line
            );
            previous = Some(*selected_index);
        }
        let omitted = entries.len().saturating_sub(selected.len());
        if omitted > 0 {
            println!("  {} entries total; {} omitted", entries.len(), omitted);
        }
    }
    println!();
    println!("Focus");
    println!(
        "  {}",
        if breadcrumb.is_empty() {
            "(document)".to_string()
        } else {
            breadcrumb.join(" › ")
        }
    );
    if let Some((start, end)) = section {
        println!("  line: {line} · section lines {start}–{end}");
    } else {
        println!("  line: {line} · section: none");
    }
    println!();
    println!("Excerpt");
    if options.no_content {
        println!("  (content omitted)");
    } else {
        let width = excerpt.end_line.to_string().len();
        let excerpt_lines = physical_lines(&excerpt.content);
        for (offset, excerpt_line) in excerpt_lines.iter().enumerate() {
            let number = excerpt.start_line + offset;
            let marker = if number == line { ">" } else { " " };
            println!(
                "{marker} {:>width$} │ {}",
                number,
                excerpt_line,
                width = width
            );
        }
        if excerpt.truncated {
            println!("  … (truncated)");
        }
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
        // As in `knapper orphans`: only hidden files are special here. A
        // folder of templates is hidden by `exclude`, not by its name.
        let orphans: Vec<&String> = graph
            .files
            .iter()
            .filter(|f| !f.starts_with('.'))
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
        // Read the template before creating anything. A vault that configured
        // a template and lost it -- renamed, not yet synced, mistyped in the
        // config -- wants to hear about it, not to be handed `# 2026-07-28`
        // and left to notice weeks later that its daily notes went blank. The
        // failure leaves the vault exactly as it was: no note, no folder.
        let body = match &config.daily_template {
            Some(relative) => {
                let template = config.vault_path.join(relative);
                let content = std::fs::read_to_string(&template).map_err(|err| {
                    anyhow!(
                        "daily_notes.template is {relative:?}, which cannot be read: {err}\n\
                         Looked in {}. Create it, correct the path, or remove the setting \
                         to get a plain dated note. Nothing was written.",
                        template.display()
                    )
                })?;
                crate::templater::expand(&content, &config.template_engine, &name, date)
            }
            // No template was ever asked for, so a bare dated note is the
            // whole of what this vault wants.
            None => format!("# {name}\n"),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
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

/// Rewrites every link to one note, for `rename` and `move`.
///
/// The regexes are built once, because the plan below runs this over every
/// note in the vault. It has to: the only honest way to know which files hold
/// a link to a note is to work out what the rewrite would do to each of them.
/// Guessing from a substring of the note's name is what silently broke
/// `[see](Old%20Note.md)` -- a link whose text contains no "Old Note" at all.
struct LinkRewriter {
    wiki: Regex,
    old_stem: String,
    new_stem: String,
    old_folder: Option<String>,
    new_folder: Option<String>,
}

impl LinkRewriter {
    fn new(
        old_stem: &str,
        new_stem: &str,
        old_folder: Option<&str>,
        new_folder: Option<&str>,
    ) -> Result<Self> {
        // A wikilink may carry a #heading or ^block-id before its |alias.
        // Missing that group left [[Note#Heading]] untouched by a rename,
        // which turned it into a broken link.
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
        Ok(Self {
            wiki,
            old_stem: old_stem.to_string(),
            new_stem: new_stem.to_string(),
            old_folder: old_folder.map(str::to_string),
            new_folder: new_folder.map(str::to_string),
        })
    }

    /// The rewritten content and how many links changed, or `None` when this
    /// file holds no link to the note.
    fn rewrite(&self, original: &str) -> Option<(String, usize)> {
        let new_stem = &self.new_stem;
        let old_folder = self.old_folder.as_deref();
        let new_folder = self.new_folder.as_deref();

        let wiki_count = self.wiki.find_iter(original).count();
        let content = self.wiki.replace_all(original, |c: &regex::Captures| {
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
        let rewritten = MD_LINK_SUB.replace_all(&content, |c: &regex::Captures| {
            let whole = c.get(0).unwrap();
            let text = &c[1];
            let href = c.get(2).or_else(|| c.get(3)).unwrap().as_str();
            let angle = c.get(2).is_some();
            let title = &c[4];

            // Images are not note links; the regex crate has no lookbehind.
            let is_image =
                whole.start() > 0 && content.as_bytes().get(whole.start() - 1) == Some(&b'!');
            if is_image || !markdown_link_targets_note(href, &self.old_stem) {
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
            let new_href = format!("{new_target}.md{anchor}");
            markdown_count += 1;
            // A path written inside <> may hold spaces as they are; a bare one
            // has to encode them, and one that was already encoded stays that
            // way rather than silently changing shape.
            if angle {
                format!("[{text}](<{new_href}>{title})")
            } else if encoded || new_href.contains(' ') {
                format!("[{text}]({}{title})", new_href.replace(' ', "%20"))
            } else {
                format!("[{text}]({new_href}{title})")
            }
        });

        (rewritten != original).then(|| (rewritten.into_owned(), wiki_count + markdown_count))
    }

    /// Every note the rewrite would change, with the content to write.
    fn plan(&self, config: &Config) -> Vec<(PathBuf, String, usize)> {
        all_notes(config)
            .into_iter()
            .filter_map(|path| {
                let original = std::fs::read_to_string(&path).ok()?;
                let (content, count) = self.rewrite(&original)?;
                Some((path, content, count))
            })
            .collect()
    }
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

    // Which files hold a link to it, worked out by rewriting each one. The
    // plan is computed before the file moves, so the note's own links are
    // read from where it still is.
    let rewriter = LinkRewriter::new(
        old_stem,
        new_stem,
        old_folder.as_deref(),
        old_folder.as_deref(),
    )?;
    let planned = rewriter.plan(config);

    if format == "text" {
        println!("Renaming: {old_relative} -> {new_relative}");
        println!("Found {} files with links to update", planned.len());
    }

    if dry_run {
        if format == "text" {
            println!("\n[DRY RUN] Would update:");
            for (path, _, count) in &planned {
                println!(
                    "  {} ({count} links)",
                    relative_path(&config.vault_path, path)
                );
            }
        } else {
            print_json(&json!({
                "old_path": old_relative, "new_path": new_relative, "dry_run": true,
                "files_to_update": planned.iter()
                    .map(|(p, _, _)| relative_path(&config.vault_path, p)).collect::<Vec<_>>()
            }));
        }
        return Ok(());
    }

    std::fs::rename(&old_path, &new_path)?;

    let mut updated_files = Vec::new();
    let mut updated_links = 0;
    for (path, content, count) in &planned {
        let path = if *path == old_path { &new_path } else { path };
        std::fs::write(path, content)?;
        updated_links += count;
        updated_files.push(relative_path(&config.vault_path, path));
        if format == "text" {
            println!(
                "  Updated {count} links in {}",
                relative_path(&config.vault_path, path)
            );
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

/// The body `knapper init` puts in the template it creates.
///
/// It is what `knapper daily` used to invent when a template was missing:
/// `{{title}}` expands to the note's date under either engine, so a fresh
/// vault gets exactly the daily note it always got. It is a starting point to
/// edit, not a suggestion about how anyone should keep a journal.
pub const DEFAULT_TEMPLATE: &str = "# {{title}}\n";

/// Create the daily-note template the generated config points at.
///
/// Returns the path if one was written. An existing file is never touched:
/// `--force` is about replacing knapper's own config, and a vault's template
/// is the user's writing, not knapper's.
fn write_default_template(dir: &Path, relative: &str) -> Result<Option<PathBuf>> {
    let target = dir.join(relative);
    if target.exists() {
        return Ok(None);
    }

    // Remember whether the folder is ours, so a failed write leaves nothing
    // of it behind either.
    let parent = target.parent().map(Path::to_path_buf);
    let made_parent = match &parent {
        Some(parent) if !parent.exists() => {
            std::fs::create_dir_all(parent)?;
            true
        }
        _ => false,
    };

    if let Err(err) = std::fs::write(&target, DEFAULT_TEMPLATE) {
        if made_parent {
            // Only ever removes the directory this call created, and only
            // while it is still empty.
            let _ = std::fs::remove_dir(parent.unwrap());
        }
        return Err(anyhow!("Could not write {}: {err}", target.display()));
    }
    Ok(Some(target))
}

pub fn init(force: bool) -> Result<()> {
    let dir = std::env::current_dir()?;
    let path = dir.join(crate::vault::CONFIG_FILENAME);
    if path.exists() && !force {
        return Err(anyhow!(
            "Config file already exists: {}\nUse --force to overwrite.",
            path.display()
        ));
    }

    // The generated config names a daily-note template, and `knapper daily`
    // fails rather than inventing a body when that template is missing. So
    // init writes the file it is about to configure, and reads the path out
    // of the config itself rather than repeating it here.
    //
    // The template goes first: if it cannot be written, no config is left
    // pointing at a file that is not there. The reverse order would report
    // success and hand the user a vault whose `daily` is already broken.
    let template = crate::vault::config_from(DEFAULT_CONFIG)
        .map_err(|err| anyhow!("the built-in config is invalid, which is a bug: {err}"))?
        .daily_template;
    let written = match &template {
        Some(relative) => write_default_template(&dir, relative)?,
        None => None,
    };

    std::fs::write(&path, DEFAULT_CONFIG)?;
    println!("Created: {}", path.display());
    if let Some(template) = written {
        println!("Created: {}", template.display());
    }
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

    let rewriter = LinkRewriter::new(&stem, &stem, old_folder.as_deref(), new_folder.as_deref())?;
    let planned = rewriter.plan(config);

    if format == "text" {
        println!("Moving: {old_relative} -> {new_relative}");
        println!("Found {} files with links to update", planned.len());
    }

    if dry_run {
        if format == "text" {
            println!("\n[DRY RUN] Would update:");
            for (path, _, count) in &planned {
                println!(
                    "  {} ({count} links)",
                    relative_path(&config.vault_path, path)
                );
            }
        } else {
            print_json(&json!({
                "old_path": old_relative, "new_path": new_relative, "dry_run": true,
                "files_to_update": planned.iter()
                    .map(|(p, _, _)| relative_path(&config.vault_path, p)).collect::<Vec<_>>()
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
    for (path, content, count) in &planned {
        let path = if *path == old_path { &new_path } else { path };
        std::fs::write(path, content)?;
        updated_links += count;
        updated_files.push(relative_path(&config.vault_path, path));
        if format == "text" {
            println!(
                "  Updated {count} links in {}",
                relative_path(&config.vault_path, path)
            );
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

#[cfg(test)]
mod focused_tests {
    use super::*;

    fn headings(source: &str) -> (Vec<FocusHeading>, Vec<Option<usize>>, Vec<usize>) {
        let lines = physical_lines(source);
        let found = markdown_headings(source, lines.len());
        let parents = heading_parents(&found);
        let (_, tops) = top_level_headings(&found, &parents);
        (found, parents, tops)
    }

    #[test]
    fn h1_children_are_the_first_outline_layer_and_breadcrumb_is_full() {
        let source = "# Root\n\n## One\n\n### Nested\nbody\n## Two\n";
        let (found, parents, tops) = headings(source);
        assert_eq!(tops, [1, 3]);
        assert_eq!(heading_path(6, &found, &parents), ["Root", "One", "Nested"]);
        let entries = outline_entries(&found, &parents, &tops, 2);
        assert_eq!(
            entries.iter().map(|entry| entry.depth).collect::<Vec<_>>(),
            [1, 2, 1]
        );
    }

    #[test]
    fn a_single_h1_without_children_is_only_the_document_title() {
        let source = "# Root\nbody\n";
        let (found, parents, tops) = headings(source);
        let (title, top) = top_level_headings(&found, &parents);
        assert_eq!(title.as_deref(), Some("Root"));
        assert!(top.is_empty());
        assert!(outline_entries(&found, &parents, &tops, 1).is_empty());
    }

    #[test]
    fn multiple_h1_headings_are_peer_first_layer_roots_without_a_title() {
        let source = "# One\nbody\n# Two\nbody\n";
        let (found, parents, tops) = headings(source);
        let (title, top) = top_level_headings(&found, &parents);
        assert!(title.is_none());
        assert_eq!(top, [0, 1]);
        assert_eq!(outline_entries(&found, &parents, &tops, 1).len(), 2);
    }

    #[test]
    fn h1_less_documents_use_the_shallowest_heading_level() {
        let source = "## One\n### nested\n## Two\n";
        let (found, parents, tops) = headings(source);
        assert_eq!(tops, [0, 2]);
        assert_eq!(top_level_headings(&found, &parents).0, None);
    }

    #[test]
    fn no_headings_and_repeated_headings_are_stable() {
        let (found, parents, tops) = headings("plain\ntext\n");
        assert!(found.is_empty());
        assert!(tops.is_empty());
        assert!(heading_path(1, &found, &parents).is_empty());

        let source = "# Same\nfirst\n# Same\nsecond\n";
        let (found, _parents, tops) = headings(source);
        assert_eq!(tops, [0, 1]);
        assert_eq!(found[0].text, found[1].text);
        assert_eq!(section_for_line(4, &found), Some((3, 4)));
    }

    #[test]
    fn outline_compaction_keeps_first_current_neighbours_and_last() {
        let source = (1..=15)
            .map(|index| format!("## Section {index}\nbody"))
            .collect::<Vec<_>>()
            .join("\n");
        let (found, parents, tops) = headings(&source);
        let entries = outline_entries(&found, &parents, &tops, 1);
        let selected = selected_outline_indices(entries.len(), Some(7));
        assert!(selected.len() <= 12);
        assert_eq!(&selected[..3], [0, 1, 2]);
        assert!(selected.contains(&5) && selected.contains(&9));
        assert_eq!(selected.last(), Some(&14));
        assert_eq!(entries.len() - selected.len(), 4);
    }

    #[test]
    fn focused_excerpt_defaults_and_explicit_limits_are_line_based() {
        let source = "a\nb\nc\nd\ne\nf\ng\n";
        let lines = physical_lines(source);
        let default = focused_excerpt(&lines, 2, 6, 4, None);
        assert_eq!((default.start_line, default.end_line), (2, 6));
        let tiny = focused_excerpt(&lines, 1, 7, 4, Some(1));
        assert_eq!(tiny.start_line, 4);
        assert_eq!(tiny.content, "d");
        assert!(tiny.truncated);
    }

    #[test]
    fn max_content_can_grow_after_focus_when_the_previous_line_does_not_fit() {
        let lines = physical_lines("long-before\nx\ny\n");
        let excerpt = focused_excerpt(&lines, 1, 3, 2, Some(3));
        assert_eq!((excerpt.start_line, excerpt.end_line), (2, 3));
        assert_eq!(excerpt.content, "x\ny");
        assert!(excerpt.truncated);
    }
}
