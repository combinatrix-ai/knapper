//! Secret-reference discovery without resolving or revealing secret values.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::LazyLock;

use anyhow::{anyhow, Result};
use regex::Regex;
use serde::Serialize;

use crate::parser::mask_noncontent;
use crate::vault::{all_notes, relative_path, resolve_path, Config};

// IDs deliberately have a small, portable alphabet. Tags follow knapper's
// Unicode-aware tag shape, but must start with a word character so malformed
// fragments such as `#/name` do not become references.
static SECRET_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"⟦secret:([A-Za-z0-9][A-Za-z0-9._:/-]{0,127})((?:[ \t]+#[\p{L}\p{N}_][\p{L}\p{N}_/-]*)*)[ \t]*⟧",
    )
    .unwrap()
});
static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#[\p{L}\p{N}_][\p{L}\p{N}_/-]*").unwrap());
static TAG_VALUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{L}\p{N}_][\p{L}\p{N}_/-]*$").unwrap());

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretRef {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub id: String,
    pub tags: Vec<String>,
}

pub fn parse_refs(path: &str, content: &str) -> Vec<SecretRef> {
    let masked = mask_noncontent(content);
    let mut refs = Vec::new();

    for (line_index, line) in masked.lines().enumerate() {
        for captures in SECRET_REF.captures_iter(line) {
            let whole = captures.get(0).expect("secret regex has a whole match");
            let id = captures.get(1).expect("secret regex has an id").as_str();
            let tag_text = captures.get(2).map_or("", |m| m.as_str());
            let tags = TAG
                .find_iter(tag_text)
                .map(|m| m.as_str().trim_start_matches('#').to_string())
                .collect();

            refs.push(SecretRef {
                path: path.to_string(),
                line: line_index + 1,
                column: line[..whole.start()].chars().count() + 1,
                id: id.to_string(),
                tags,
            });
        }
    }
    refs
}

fn collect(config: &Config, file: Option<&str>) -> Result<Vec<SecretRef>> {
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

fn print(refs: &[SecretRef], format: &str) {
    match format {
        "json" => println!("{}", serde_json::to_string_pretty(refs).unwrap()),
        "paths" => {
            let paths: BTreeSet<&str> = refs.iter().map(|r| r.path.as_str()).collect();
            paths.iter().for_each(|path| println!("{path}"));
        }
        _ => refs.iter().for_each(|r| {
            let tags = r
                .tags
                .iter()
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join(" ");
            if tags.is_empty() {
                println!("{}:{}:{}  {}", r.path, r.line, r.column, r.id);
            } else {
                println!("{}:{}:{}  {}  {tags}", r.path, r.line, r.column, r.id);
            }
        }),
    }
}

pub fn refs(config: &Config, file: Option<&str>, format: &str) -> Result<()> {
    let found = collect(config, file)?;
    print(&found, format);
    Ok(())
}

pub fn find(config: &Config, tags: &[String], format: &str) -> Result<()> {
    let normalized: Result<Vec<&str>> = tags
        .iter()
        .map(|tag| {
            let tag = tag.strip_prefix('#').unwrap_or(tag);
            if TAG_VALUE.is_match(tag) {
                Ok(tag)
            } else {
                Err(anyhow!("Invalid secret tag: {tag:?}"))
            }
        })
        .collect();
    let normalized = normalized?;
    let found: Vec<_> = collect(config, None)?
        .into_iter()
        .filter(|reference| {
            normalized
                .iter()
                .all(|tag| reference.tags.iter().any(|candidate| candidate == tag))
        })
        .collect();
    print(&found, format);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_locations_and_unicode_tags() {
        let refs = parse_refs(
            "Note.md",
            "before ⟦secret:db/prod #database #日本語⟧\n日本語 ⟦secret:key-2⟧",
        );
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].line, 1);
        assert_eq!(refs[0].column, 8);
        assert_eq!(refs[0].id, "db/prod");
        assert_eq!(refs[0].tags, ["database", "日本語"]);
        assert_eq!(refs[1].column, 5);
    }

    #[test]
    fn ignores_noncontent_and_malformed_markers() {
        let refs = parse_refs(
            "Note.md",
            "`⟦secret:inline #x⟧`\n%% ⟦secret:comment #x⟧ %%\n```\n⟦secret:fenced #x⟧\n```\n⟦secret:has space #x⟧\n⟦secret:ok #x⟧",
        );
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].id, "ok");
        assert_eq!(refs[0].line, 7);
    }
}
