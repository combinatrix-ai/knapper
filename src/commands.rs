//! Command implementations.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::graph::build_link_graph;
use crate::note::parse_note;
use crate::parser::{extract_links, mask_noncontent};
use crate::vault::{all_notes, relative_path, resolve_path, Config};

/// Every JSON value knapper prints goes through here, so non-ASCII stays
/// readable rather than becoming \uXXXX escapes.
fn print_json(value: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

/// The lines around a match, as `-A`/`-B` ask for. `line` is 1-based.
fn context_window(lines: &[&str], line: usize, before: usize, after: usize) -> Option<String> {
    (before > 0 || after > 0).then(|| {
        let start = (line - 1).saturating_sub(before);
        let end = (line + after).min(lines.len());
        lines[start..end].join("\n")
    })
}

pub fn links(config: &Config, file: &str, format: &str, before: usize, after: usize) -> Result<()> {
    let path = resolve_path(&config.vault_path, file);
    if !path.exists() {
        return Err(anyhow!("File not found: {}", path.display()));
    }

    let content = std::fs::read_to_string(&path)?;
    // Scan masked lines so a link inside a code fence or a %%comment%% is not
    // reported; masking preserves line numbers.
    let masked = mask_noncontent(&content);

    let lines: Vec<&str> = content.lines().collect();
    let mut results = Vec::new();
    for (index, line) in masked.lines().enumerate() {
        for target in extract_links(line) {
            let mut entry = json!({"target": target, "line": index + 1});
            if let Some(context) = context_window(&lines, index + 1, before, after) {
                entry["context"] = json!(context);
            }
            results.push(entry);
        }
    }

    match format {
        "json" => print_json(&serde_json::Value::Array(results)),
        "paths" => {
            let mut seen = std::collections::BTreeSet::new();
            for r in &results {
                let target = r["target"].as_str().unwrap_or_default();
                if seen.insert(target.to_string()) {
                    println!("{target}");
                }
            }
        }
        _ => {
            for r in &results {
                println!("\n{} (line {})", r["target"].as_str().unwrap(), r["line"]);
                if let Some(context) = r["context"].as_str() {
                    context.split('\n').for_each(|l| println!("  {l}"));
                }
            }
        }
    }
    Ok(())
}

pub fn backlinks(
    config: &Config,
    file: &str,
    format: &str,
    before: usize,
    after: usize,
) -> Result<()> {
    let path = resolve_path(&config.vault_path, file);
    let target_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let relative = relative_path(&config.vault_path, &path);
    let without_ext = relative
        .rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| relative.clone());

    let mut results = Vec::new();
    for source in all_notes(config) {
        if source == path {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&source) else {
            continue;
        };
        let masked = mask_noncontent(&content);
        let lines: Vec<&str> = content.lines().collect();

        for (index, line) in masked.lines().enumerate() {
            for candidate in extract_links(line) {
                let basename = Path::new(&candidate)
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if candidate == target_name || candidate == without_ext || basename == target_name {
                    let mut entry = json!({
                        "source": relative_path(&config.vault_path, &source),
                        "line": index + 1,
                    });
                    if let Some(context) = context_window(&lines, index + 1, before, after) {
                        entry["context"] = json!(context);
                    }
                    results.push(entry);
                }
            }
        }
    }

    match format {
        "json" => print_json(&serde_json::Value::Array(results)),
        "paths" => {
            let mut seen = std::collections::BTreeSet::new();
            for r in &results {
                let source = r["source"].as_str().unwrap_or_default();
                if seen.insert(source.to_string()) {
                    println!("{source}");
                }
            }
        }
        _ => {
            for r in &results {
                println!("\n{} (line {})", r["source"].as_str().unwrap(), r["line"]);
                if let Some(context) = r["context"].as_str() {
                    context.split('\n').for_each(|l| println!("  {l}"));
                }
            }
        }
    }
    Ok(())
}

pub fn orphans(config: &Config, format: &str, include_special: bool) -> Result<()> {
    let graph = build_link_graph(config);

    let orphans: Vec<String> = graph
        .files
        .iter()
        .filter(|f| include_special || (!f.starts_with("Templates/") && !f.starts_with('.')))
        .filter(|f| graph.incoming.get(*f).map_or(true, |i| i.is_empty()))
        .cloned()
        .collect();

    match format {
        "json" => print_json(&json!(orphans)),
        "paths" => orphans.iter().for_each(|f| println!("{f}")),
        _ => {
            println!("Found {} orphan notes:\n", orphans.len());
            orphans.iter().for_each(|f| println!("  {f}"));
        }
    }
    Ok(())
}

pub fn hubs(config: &Config, limit: usize, format: &str) -> Result<()> {
    let graph = build_link_graph(config);

    let mut ranked: Vec<(String, usize)> = graph
        .incoming
        .iter()
        .map(|(file, sources)| (file.clone(), sources.len()))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(limit);

    match format {
        "json" => print_json(&json!(ranked
            .iter()
            .map(|(f, c)| json!({"file": f, "incoming_links": c}))
            .collect::<Vec<_>>())),
        "paths" => ranked.iter().for_each(|(f, _)| println!("{f}")),
        _ => {
            println!("Top {} hub notes:\n", ranked.len());
            for (file, count) in &ranked {
                println!("  {count:3} links <- {file}");
            }
        }
    }
    Ok(())
}

pub fn broken_links(config: &Config, format: &str) -> Result<()> {
    let graph = build_link_graph(config);
    let total: usize = graph.broken.values().map(|v| v.len()).sum();

    match format {
        "json" => {
            let items: Vec<_> = graph
                .broken
                .iter()
                .map(|(file, links)| json!({"file": file, "broken_links": links}))
                .collect();
            print_json(&json!(items));
        }
        "paths" => graph.broken.keys().for_each(|f| println!("{f}")),
        _ => {
            if total == 0 {
                println!("No broken links found!");
                return Ok(());
            }
            println!(
                "Found {total} broken links in {} files:\n",
                graph.broken.len()
            );
            for (file, links) in &graph.broken {
                println!("  {file}:");
                for link in links {
                    println!("    -> [[{link}]]");
                }
            }
        }
    }
    Ok(())
}

pub fn tags(config: &Config, file: Option<&str>, find: Option<&str>, format: &str) -> Result<()> {
    if let Some(needle) = find {
        let mut matched = Vec::new();
        for path in all_notes(config) {
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if parse_note(&path, &content).tags.iter().any(|t| t == needle) {
                matched.push(relative_path(&config.vault_path, &path));
            }
        }
        matched.sort();
        match format {
            "json" => print_json(&json!(matched)),
            _ => matched.iter().for_each(|f| println!("{f}")),
        }
        return Ok(());
    }

    if let Some(file) = file {
        let path = resolve_path(&config.vault_path, file);
        let content = std::fs::read_to_string(&path)?;
        let tags = parse_note(&path, &content).tags;
        match format {
            "json" => print_json(&json!(tags)),
            _ => tags.iter().for_each(|t| println!("{t}")),
        }
        return Ok(());
    }

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for path in all_notes(config) {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for tag in parse_note(&path, &content).tags {
            *counts.entry(tag).or_insert(0) += 1;
        }
    }

    match format {
        "json" => print_json(&json!(counts)),
        _ => {
            let mut ranked: Vec<_> = counts.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            for (tag, count) in ranked {
                println!("{tag} ({count})");
            }
        }
    }
    Ok(())
}
