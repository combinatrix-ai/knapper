//! `query` - one filter over every note.
//!
//! `orphans`, `hubs` and `broken-links` are this with the filter fixed.
//! Frontmatter keys, inline fields and computed fields share one namespace,
//! which is what makes a typed relation queryable.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Local};
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::graph::{build_link_graph, LinkGraph};
use crate::note::parse_note;
use crate::vault::{all_notes, relative_path, Config};

pub const COMPUTED_FIELDS: &[&str] = &[
    "path", "title", "ext", "inlinks", "outlinks", "broken", "tags", "words", "chars", "lines",
    "modified", "age",
];

/// Fields needing the whole link graph, which is the expensive part.
const GRAPH_FIELDS: &[&str] = &["inlinks", "outlinks", "broken"];

#[derive(Debug, Clone)]
pub struct Predicate {
    pub field: String,
    pub op: String,
    pub value: Option<String>,
    pub negate: bool,
}

const OPERATORS: &[&str] = &[">=", "<=", "!=", "=", ">", "<", "~"];

pub fn parse_predicate(expr: &str) -> Result<Predicate> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err(anyhow!("--where needs an expression"));
    }

    if let Some(rest) = expr.strip_prefix('!') {
        let rest = rest.trim();
        if rest.is_empty() {
            return Err(anyhow!("--where '!' needs a field name"));
        }
        if OPERATORS.iter().any(|op| rest.contains(op)) {
            return Err(anyhow!(
                "--where {expr:?}: '!' negates a bare field name; use '!=' to compare instead"
            ));
        }
        return Ok(Predicate {
            field: rest.to_string(),
            op: "exists".into(),
            value: None,
            negate: true,
        });
    }

    for op in OPERATORS {
        if let Some(index) = expr.find(op) {
            if index > 0 {
                return Ok(Predicate {
                    field: expr[..index].trim().to_string(),
                    op: (*op).to_string(),
                    value: Some(expr[index + op.len()..].trim().to_string()),
                    negate: false,
                });
            }
        }
    }

    Ok(Predicate {
        field: expr.to_string(),
        op: "exists".into(),
        value: None,
        negate: false,
    })
}

fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn flatten(value: &Value) -> String {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|i| {
                i.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| i.to_string())
            })
            .collect::<Vec<_>>()
            .join(" "),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn matches(record: &serde_json::Map<String, Value>, p: &Predicate) -> bool {
    let actual = record.get(&p.field);

    if p.op == "exists" {
        let present = match actual {
            None | Some(Value::Null) => false,
            Some(Value::String(s)) => !s.is_empty(),
            Some(Value::Array(a)) => !a.is_empty(),
            Some(Value::Object(o)) => !o.is_empty(),
            _ => true,
        };
        return present != p.negate;
    }

    let Some(actual) = actual else { return false };
    if actual.is_null() {
        return false;
    }
    let wanted = p.value.as_deref().unwrap_or_default();

    if p.op == "~" {
        return flatten(actual)
            .to_lowercase()
            .contains(&wanted.to_lowercase());
    }

    if let (Some(left), Ok(right)) = (as_number(actual), wanted.parse::<f64>()) {
        return match p.op.as_str() {
            ">" => left > right,
            "<" => left < right,
            ">=" => left >= right,
            "<=" => left <= right,
            "=" => left == right,
            "!=" => left != right,
            _ => false,
        };
    }

    let left = flatten(actual).to_lowercase();
    let right = wanted.to_lowercase();

    match p.op.as_str() {
        // Ordering on text is defined, and useful for dates.
        ">" => left > right,
        "<" => left < right,
        ">=" => left >= right,
        "<=" => left <= right,
        _ => {
            let equal = match actual {
                Value::Array(items) => items.iter().any(|i| flatten(i).to_lowercase() == right),
                _ => left == right,
            };
            if p.op == "=" {
                equal
            } else {
                !equal
            }
        }
    }
}

fn build_record(
    path: &Path,
    relative: &str,
    graph: Option<&LinkGraph>,
    today: DateTime<Local>,
) -> Option<serde_json::Map<String, Value>> {
    let content = std::fs::read_to_string(path).ok()?;
    let note = parse_note(path, &content);

    let mut record = serde_json::Map::new();
    record.insert("path".into(), json!(relative));
    record.insert("title".into(), json!(note.title));
    record.insert(
        "ext".into(),
        json!(path.extension().and_then(|e| e.to_str()).unwrap_or("")),
    );
    let mut tags = note.tags.clone();
    tags.sort();
    record.insert("tags".into(), json!(tags));
    record.insert("chars".into(), json!(note.content.chars().count()));
    record.insert(
        "words".into(),
        json!(note.content.split_whitespace().count()),
    );
    record.insert(
        "lines".into(),
        json!(if note.content.is_empty() {
            0
        } else {
            note.content.matches('\n').count() + 1
        }),
    );

    if let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) {
        let modified: DateTime<Local> = modified.into();
        record.insert(
            "modified".into(),
            json!(modified.format("%Y-%m-%d").to_string()),
        );
        record.insert(
            "age".into(),
            json!((today.date_naive() - modified.date_naive()).num_days()),
        );
    }

    if let Some(graph) = graph {
        record.insert(
            "inlinks".into(),
            json!(graph.incoming.get(relative).map_or(0, |s| s.len())),
        );
        record.insert(
            "outlinks".into(),
            json!(graph.outgoing.get(relative).map_or(0, |s| s.len())),
        );
        record.insert(
            "broken".into(),
            json!(graph.broken.get(relative).map_or(0, |v| v.len())),
        );
    }

    // The note's own fields come last so a computed name always wins; a note
    // cannot shadow "path" by declaring a property called path.
    for (key, value) in &note.inline_fields {
        record.entry(key.clone()).or_insert_with(|| json!(value));
    }
    for (key, value) in &note.frontmatter {
        let Some(key) = key.as_str() else { continue };
        if key == "tags" {
            continue;
        }
        record
            .entry(key.to_string())
            .or_insert_with(|| yaml_to_json(value));
    }

    Some(record)
}

pub fn yaml_mapping_to_json(map: &serde_yaml::Mapping) -> Value {
    Value::Object(
        map.iter()
            .filter_map(|(k, v)| k.as_str().map(|k| (k.to_string(), yaml_to_json(v))))
            .collect(),
    )
}

fn yaml_to_json(value: &serde_yaml::Value) -> Value {
    match value {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => json!(b),
        serde_yaml::Value::Number(n) => n
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        serde_yaml::Value::String(s) => json!(s),
        serde_yaml::Value::Sequence(items) => {
            Value::Array(items.iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(map) => Value::Object(
            map.iter()
                .filter_map(|(k, v)| k.as_str().map(|k| (k.to_string(), yaml_to_json(v))))
                .collect(),
        ),
        _ => Value::Null,
    }
}

#[derive(Default)]
pub struct QueryArgs<'a> {
    pub where_: &'a [String],
    pub fields: &'a [String],
    pub from: Option<&'a str>,
    pub sort: Option<&'a str>,
    pub limit: Option<usize>,
    pub format: &'a str,
}

pub fn query(config: &Config, args: &QueryArgs) -> Result<()> {
    let predicates: Vec<Predicate> = args
        .where_
        .iter()
        .map(|w| parse_predicate(w))
        .collect::<Result<_>>()?;

    let mut paths = all_notes(config);
    if let Some(from) = args.from {
        let prefix = from.trim_end_matches('/');
        paths.retain(|p| relative_path(&config.vault_path, p).starts_with(prefix));
    }

    // The link graph reads every note, so build it only when something asks
    // for a field that needs it.
    let mut names: Vec<&str> = predicates.iter().map(|p| p.field.as_str()).collect();
    names.extend(args.fields.iter().map(String::as_str));
    if let Some(sort) = args.sort {
        names.push(sort.split(':').next().unwrap_or(sort));
    }
    let graph = names
        .iter()
        .any(|n| GRAPH_FIELDS.contains(n))
        .then(|| build_link_graph(config));

    let today = Local::now();
    let mut records: Vec<serde_json::Map<String, Value>> = paths
        .par_iter()
        .filter_map(|path| {
            let relative = relative_path(&config.vault_path, path);
            build_record(path, &relative, graph.as_ref(), today)
        })
        .filter(|r| predicates.iter().all(|p| matches(r, p)))
        .collect();

    // Sort by path first so ties have a defined order, then stably by the
    // requested key. Reversing the vector instead would flip ties too, which
    // is not what a stable descending sort means.
    records.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));

    if let Some(spec) = args.sort {
        let (name, direction) = spec.split_once(':').unwrap_or((spec, "asc"));
        let descending = direction.eq_ignore_ascii_case("desc");
        records.sort_by(|a, b| {
            let key = |r: &serde_json::Map<String, Value>| {
                let v = r.get(name).cloned().unwrap_or(Value::Null);
                (v.is_null(), as_number(&v).unwrap_or(0.0), flatten(&v))
            };
            let (ax, ay, az) = key(a);
            let (bx, by, bz) = key(b);
            let ordering = ax
                .cmp(&bx)
                .then(ay.partial_cmp(&by).unwrap_or(std::cmp::Ordering::Equal))
                .then(az.cmp(&bz));
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }

    if let Some(limit) = args.limit {
        records.truncate(limit);
    }

    output(&records, args.fields, args.format);
    Ok(())
}

fn present(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|i| {
                i.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| i.to_string())
            })
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn output(records: &[serde_json::Map<String, Value>], fields: &[String], format: &str) {
    match format {
        "json" => {
            let items: Vec<Value> = if fields.is_empty() {
                records.iter().cloned().map(Value::Object).collect()
            } else {
                records
                    .iter()
                    .map(|r| {
                        let mut out = serde_json::Map::new();
                        out.insert("path".into(), r["path"].clone());
                        for f in fields {
                            out.insert(f.clone(), r.get(f).cloned().unwrap_or(Value::Null));
                        }
                        Value::Object(out)
                    })
                    .collect()
            };
            println!("{}", serde_json::to_string_pretty(&items).unwrap());
        }
        "paths" => records
            .iter()
            .for_each(|r| println!("{}", r["path"].as_str().unwrap_or_default())),
        _ => {
            if records.is_empty() {
                println!("No notes matched.");
                return;
            }
            if fields.is_empty() {
                records
                    .iter()
                    .for_each(|r| println!("{}", r["path"].as_str().unwrap_or_default()));
                println!("\n{} notes", records.len());
                return;
            }

            let mut columns = vec!["path".to_string()];
            columns.extend(fields.iter().cloned());
            let rows: Vec<Vec<String>> = records
                .iter()
                .map(|r| columns.iter().map(|c| present(r.get(c))).collect())
                .collect();
            let widths: Vec<usize> = columns
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    rows.iter()
                        .map(|row| row[i].chars().count())
                        .chain(std::iter::once(c.chars().count()))
                        .max()
                        .unwrap_or(0)
                })
                .collect();

            let pad = |s: &str, w: usize| {
                let mut out = s.to_string();
                for _ in s.chars().count()..w {
                    out.push(' ');
                }
                out
            };
            println!(
                "{}",
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| pad(c, widths[i]))
                    .collect::<Vec<_>>()
                    .join("  ")
            );
            println!(
                "{}",
                widths
                    .iter()
                    .map(|w| "-".repeat(*w))
                    .collect::<Vec<_>>()
                    .join("  ")
            );
            for row in &rows {
                println!(
                    "{}",
                    row.iter()
                        .enumerate()
                        .map(|(i, c)| pad(c, widths[i]))
                        .collect::<Vec<_>>()
                        .join("  ")
                );
            }
            println!("\n{} notes", records.len());
        }
    }
}

/// `fields` - what a given vault offers, computed and declared.
pub fn fields(config: &Config, format: &str) -> Result<()> {
    let mut declared: BTreeMap<String, usize> = BTreeMap::new();

    for path in all_notes(config) {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let note = parse_note(&path, &content);
        let keys = note
            .frontmatter
            .keys()
            .filter_map(|k| k.as_str().map(str::to_string))
            .chain(note.inline_fields.keys().cloned());
        for key in keys {
            if key != "tags" {
                *declared.entry(key).or_insert(0) += 1;
            }
        }
    }

    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "computed": COMPUTED_FIELDS,
                "declared": declared,
            }))
            .unwrap()
        );
        return Ok(());
    }

    println!("Computed fields");
    println!("  {}", COMPUTED_FIELDS.join(", "));
    println!("\nDeclared by notes (frontmatter and inline fields)");
    let mut ranked: Vec<_> = declared.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (key, count) in ranked {
        println!("  {key:24} {count}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(expr: &str) -> (String, String, Option<String>, bool) {
        let p = parse_predicate(expr).unwrap_or_else(|e| panic!("{expr:?}: {e}"));
        (p.field, p.op, p.value, p.negate)
    }

    /// Frontmatter keys, Dataview inline fields and computed fields share one
    /// namespace, so one predicate grammar covers all three.
    #[test]
    fn every_operator_form_parses() {
        for (expr, field, op, value, negate) in [
            ("status=open", "status", "=", Some("open"), false),
            ("status!=done", "status", "!=", Some("done"), false),
            ("inlinks>3", "inlinks", ">", Some("3"), false),
            ("inlinks>=3", "inlinks", ">=", Some("3"), false),
            ("age<=30", "age", "<=", Some("30"), false),
            ("tags~research", "tags", "~", Some("research"), false),
            // A bare name asks whether the field is present at all.
            ("status", "status", "exists", None, false),
            ("!status", "status", "exists", None, true),
            (" status = open ", "status", "=", Some("open"), false),
        ] {
            assert_eq!(
                parsed(expr),
                (
                    field.to_string(),
                    op.to_string(),
                    value.map(str::to_string),
                    negate
                ),
                "input: {expr:?}"
            );
        }
    }

    /// The longer operators must win, or `>=` would parse as `>` with a
    /// value beginning `=`.
    #[test]
    fn two_character_operators_are_not_split() {
        assert_eq!(parsed("a>=1").1, ">=");
        assert_eq!(parsed("a<=1").1, "<=");
        assert_eq!(parsed("a!=1").1, "!=");
    }

    #[test]
    fn empty_and_malformed_expressions_are_refused() {
        for expr in ["", "   ", "!", "!a=b"] {
            assert!(
                parse_predicate(expr).is_err(),
                "should have been refused: {expr:?}"
            );
        }
    }
}
