//! Read-only DQL rules: select notes which violate a vault convention.
use crate::vault::Config;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub fn run(
    config: &Config,
    checks: &[String],
    note_paths: &[String],
    selection: Option<&BTreeSet<String>>,
) -> Result<Vec<Value>> {
    let rules: Vec<_> = config
        .lint_dataview
        .iter()
        .filter(|(name, rule)| {
            if checks.is_empty() {
                rule.enabled
            } else {
                checks
                    .iter()
                    .any(|c| c == "dataview" || c == &format!("dataview:{name}"))
            }
        })
        .collect();
    if rules.is_empty() {
        return Ok(Vec::new());
    }
    let timezone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
    // Always query the whole scoped vault: file selection affects reporting,
    // never aggregate/filter inputs. All rules share one fresh snapshot.
    let mut input = crate::dql::snapshot(config, None, "", &timezone, ".obsidian")?;
    let mut issues = Vec::new();
    for (name, rule) in rules {
        let result = (|| -> Result<Vec<Value>> {
            input["query"] = json!(rule.query);
            let output = crate::dql::evaluate(&input, 256, 10)?;
            if output["type"] != "list" {
                return Err(anyhow!("lint requires a LIST query selecting note links or exact file.path strings"));
            }
            if output["diagnostics"].as_array().is_some_and(|ds| ds.iter().any(|d| d["errors"].as_array().is_some_and(|e| !e.is_empty()))) {
                return Err(anyhow!("DQL row evaluation failed: {}", output["diagnostics"]));
            }
            let mut paths = BTreeSet::new();
            for value in output["values"].as_array().ok_or_else(|| anyhow!("DQL LIST returned no values array"))? {
                let path = if value["type"] == "link" { value["path"].as_str() } else { value.as_str() }
                    .ok_or_else(|| anyhow!("LIST rows must be note links or exact file.path strings; use LIST FROM ... or LIST WITHOUT ID file.path ..."))?;
                if !note_paths.iter().any(|p| p == path) {
                    return Err(anyhow!("LIST returned an unindexed note path: {path}"));
                }
                if selection.is_none_or(|set| set.contains(path)) { paths.insert(path.to_string()); }
            }
            Ok(paths.into_iter().map(|path| json!({"type":"dataview", "rule":name, "file":path, "detail":rule.message, "severity":rule.severity})).collect())
        })().with_context(|| format!("Dataview lint rule `{name}`"))?;
        issues.extend(result);
    }
    Ok(issues)
}
