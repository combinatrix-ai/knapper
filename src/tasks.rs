//! Tasks: finding, filtering, and the several ways a vault writes one.
//!
//! Three notations land in the same `Task`: markdown checkboxes with the
//! Obsidian Tasks emoji dates, outliner keywords from Roam and Logseq, and
//! org-mode TODO states. Planning lines are folded into the text using the
//! emoji markers, so the date filters need no per-flavor branches.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use regex::Regex;
use serde_json::json;

use crate::org;
use crate::vault::{all_notes, is_org, relative_path, Config};

static CHECKBOX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)-\s*\[([^\]])\]\s*(.+)$").unwrap());

// Roam writes task state as a macro. That cannot occur in ordinary markdown,
// so it is always recognised.
static ROAM_TASK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*-\s*\{\{\[\[(TODO|DOING|DONE|CANCELLED|CANCELED)\]\]\}\}\s*(.*)$").unwrap()
});

// Logseq writes a bare keyword instead, which is indistinguishable from prose,
// so it applies only under flavor: logseq.
static LOGSEQ_TASK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*-\s+(TODO|DOING|NOW|LATER|WAITING|IN-PROGRESS|DONE|CANCELED|CANCELLED)\s+(.+)$",
    )
    .unwrap()
});
static LOGSEQ_PLANNING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(SCHEDULED|DEADLINE):\s*<(\d{4}-\d{2}-\d{2})").unwrap());

static RECURRENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)🔁\s*every\s+(\d+)\s+(day|days|week|weeks|month|months|year|years)\b").unwrap()
});
static TAG_IN_TASK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#([\w/-]+)").unwrap());

const PRIORITY_MARKERS: &[(&str, &str)] = &[("⏫", "high"), ("🔼", "medium"), ("🔽", "low")];

#[derive(Debug, Clone, Default)]
pub struct Task {
    pub file: String,
    pub line: usize,
    pub text: String,
    pub done: bool,
    pub status: String,
    pub status_char: char,
    pub closed: bool,
    pub due_date: Option<String>,
    pub created_date: Option<String>,
    pub done_date: Option<String>,
    pub scheduled_date: Option<String>,
    pub start_date: Option<String>,
    pub priority: Option<String>,
    pub recurrence: Option<(u32, String)>,
    pub tags: Vec<String>,
    /// The heading hierarchy above the task, root first. Filled always; shown
    /// only when asked for, as it is display detail rather than task data.
    pub breadcrumbs: Vec<String>,
    /// Surrounding lines, when the caller asked for context.
    pub context: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub char: char,
    pub closed: bool,
    pub date_format: Option<String>,
}

/// The built-in statuses, merged with whatever the config overrides or adds.
pub fn resolve_statuses(config: &Config) -> BTreeMap<String, Status> {
    let builtin = |char, closed, date_format: Option<&str>| Status {
        char,
        closed,
        date_format: date_format.map(str::to_string),
    };
    let mut merged = BTreeMap::from([
        ("open".to_string(), builtin(' ', false, None)),
        (
            "done".to_string(),
            builtin('x', true, Some("✅ YYYY-MM-DD")),
        ),
        (
            "cancel".to_string(),
            builtin('-', true, Some("❌ YYYY-MM-DD")),
        ),
        ("wip".to_string(), builtin('/', false, None)),
    ]);

    // The legacy done_date pair stays authoritative for `done`.
    if let Some(done) = merged.get_mut("done") {
        done.date_format = config
            .tasks_done_date
            .then(|| config.tasks_done_date_format.clone());
    }

    for (name, attrs) in &config.tasks_statuses {
        match merged.get_mut(name) {
            Some(existing) => {
                if let Some(c) = attrs.char {
                    existing.char = c;
                }
                if let Some(c) = attrs.closed {
                    existing.closed = c;
                }
                if attrs.date_format_set {
                    existing.date_format = attrs.date_format.clone();
                }
            }
            // A new status needs a checkbox character to be addressable.
            None => {
                if let Some(c) = attrs.char {
                    merged.insert(
                        name.clone(),
                        Status {
                            char: c,
                            closed: attrs.closed.unwrap_or(false),
                            date_format: attrs.date_format.clone(),
                        },
                    );
                }
            }
        }
    }
    merged
}

fn status_name(ch: char, statuses: &BTreeMap<String, Status>) -> String {
    statuses
        .iter()
        .find(|(_, s)| s.char.eq_ignore_ascii_case(&ch))
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| "unknown".into())
}

fn first_date(text: &str, patterns: &[&str]) -> Option<String> {
    for p in patterns {
        if let Some(c) = Regex::new(p).ok()?.captures(text) {
            return Some(c[1].to_string());
        }
    }
    None
}

pub fn parse_due_date(text: &str) -> Option<String> {
    first_date(
        text,
        &[
            r"📅\s*(\d{4}-\d{2}-\d{2})",
            r"due:(\d{4}-\d{2}-\d{2})",
            r"@due\((\d{4}-\d{2}-\d{2})\)",
        ],
    )
}

pub fn parse_done_date(text: &str) -> Option<String> {
    first_date(
        text,
        &[
            r"✅\s*(\d{4}-\d{2}-\d{2})",
            r"done:(\d{4}-\d{2}-\d{2})",
            r"@done\((\d{4}-\d{2}-\d{2})\)",
        ],
    )
}

pub fn parse_created_date(text: &str) -> Option<String> {
    first_date(
        text,
        &[
            r"➕\s*(\d{4}-\d{2}-\d{2})",
            r"created:(\d{4}-\d{2}-\d{2})",
            r"@created\((\d{4}-\d{2}-\d{2})\)",
        ],
    )
}

pub fn parse_scheduled_date(text: &str) -> Option<String> {
    first_date(
        text,
        &[
            r"⏳\s*(\d{4}-\d{2}-\d{2})",
            r"scheduled:(\d{4}-\d{2}-\d{2})",
        ],
    )
}

pub fn parse_start_date(text: &str) -> Option<String> {
    first_date(
        text,
        &[r"🛫\s*(\d{4}-\d{2}-\d{2})", r"start:(\d{4}-\d{2}-\d{2})"],
    )
}

pub fn parse_priority(text: &str) -> Option<String> {
    PRIORITY_MARKERS
        .iter()
        .find(|(marker, _)| text.contains(marker))
        .map(|(_, name)| name.to_string())
}

pub fn parse_recurrence(text: &str) -> Option<(u32, String)> {
    RECURRENCE.captures(text).and_then(|c| {
        let interval = c[1].parse().ok()?;
        let mut unit = c[2].to_lowercase();
        if !unit.ends_with('s') {
            unit.push('s');
        }
        Some((interval, unit))
    })
}

/// Parse YYYY-MM-DD, or reject the value outright.
///
/// Date filters used to fall back to an unbounded range when the value did not
/// parse, which silently returned every task instead of reporting bad input.
pub fn require_ymd(value: &str, flag: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").map_err(|_| {
        anyhow!(
            "Invalid value: {flag} expects a YYYY-MM-DD date, got {value:?}. \
             Relative dates such as 'today' or '+3d' are not supported."
        )
    })
}

type Range = (NaiveDate, NaiveDate);

fn inclusive_range(
    from: Option<&str>,
    to: Option<&str>,
    from_flag: &str,
    to_flag: &str,
) -> Result<Option<Range>> {
    if from.is_none() && to.is_none() {
        return Ok(None);
    }
    let start = match from {
        Some(v) => require_ymd(v, from_flag)?,
        None => NaiveDate::MIN,
    };
    let end = match to {
        Some(v) => require_ymd(v, to_flag)?
            .succ_opt()
            .unwrap_or(NaiveDate::MAX),
        None => NaiveDate::MAX,
    };
    Ok(Some((start, end)))
}

fn in_range(date: &Option<String>, range: &Option<Range>) -> bool {
    let Some((start, end)) = range else {
        return true;
    };
    let Some(value) = date else { return false };
    match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        Ok(d) => d >= *start && d < *end,
        Err(_) => false,
    }
}

/// (line index, checkbox char, text) for the outliner notations.
fn outliner_candidates(lines: &[&str], flavor: &str) -> Vec<(usize, char, String)> {
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let caps = ROAM_TASK.captures(line).or_else(|| {
            (flavor == "logseq")
                .then(|| LOGSEQ_TASK.captures(line))
                .flatten()
        });
        let Some(caps) = caps else { continue };

        let state = caps[1].to_uppercase();
        let checkbox = match state.as_str() {
            "DONE" => 'x',
            "CANCELLED" | "CANCELED" => '-',
            "DOING" | "NOW" | "IN-PROGRESS" => '/',
            _ => ' ',
        };

        // Fold any planning lines belonging to this block into the text using
        // the emoji markers the date parsers already understand.
        let mut parts = vec![caps[2].trim().to_string()];
        for following in lines.iter().skip(index + 1).take(3) {
            match LOGSEQ_PLANNING.captures(following) {
                Some(p) => {
                    let marker = if &p[1] == "DEADLINE" { "📅" } else { "⏳" };
                    parts.push(format!("{marker} {}", &p[2]));
                }
                None => {
                    let trimmed = following.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with(':') {
                        break;
                    }
                }
            }
        }
        out.push((
            index,
            checkbox,
            parts
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        ));
    }
    out
}

fn org_candidates(content: &str, lines: &[&str]) -> Vec<(usize, char, String)> {
    let doc = org::parse_org(content);
    doc.headings
        .into_iter()
        .filter_map(|heading| {
            let todo = heading.todo?;
            let is_done = org::ORG_DONE_STATES.contains(&todo.as_str());
            if !is_done && !org::ORG_OPEN_STATES.contains(&todo.as_str()) {
                return None;
            }

            let mut parts = vec![heading.text.clone()];
            // Planning sits on the lines under the heading.
            for following in lines.iter().skip(heading.line).take(3) {
                if following.trim_start().starts_with('*') {
                    break;
                }
                for (kind, date) in org::parse_org_planning(following) {
                    let marker = match kind.as_str() {
                        "deadline" => "📅",
                        "scheduled" => "⏳",
                        _ => "✅",
                    };
                    parts.push(format!("{marker} {date}"));
                }
            }
            for tag in &heading.tags {
                parts.push(format!("#{tag}"));
            }
            if let Some(p) = &heading.priority {
                parts.push(
                    match p.as_str() {
                        "A" => "⏫",
                        "B" => "🔼",
                        _ => "🔽",
                    }
                    .to_string(),
                );
            }

            Some((
                heading.line - 1,
                if is_done { 'x' } else { ' ' },
                parts
                    .into_iter()
                    .filter(|p| !p.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            ))
        })
        .collect()
}

#[derive(Debug, Default, Clone)]
pub struct Filters<'a> {
    pub include_done: bool,
    pub only_done: bool,
    pub recurring_only: bool,
    pub overdue: bool,
    pub has_date: bool,
    pub file: Option<&'a str>,
    pub exclude: &'a [String],
    pub tag: Option<&'a str>,
    pub status: &'a [String],
    pub due_on: Option<&'a str>,
    pub due_from: Option<&'a str>,
    pub due_to: Option<&'a str>,
    pub created_from: Option<&'a str>,
    pub created_to: Option<&'a str>,
    pub done_from: Option<&'a str>,
    pub done_to: Option<&'a str>,
    pub context_before: usize,
    pub context_after: usize,
}

/// How to render what was found. Separate from Filters because none of it
/// changes which tasks match.
#[derive(Debug, Default, Clone)]
pub struct Display {
    pub breadcrumbs: bool,
    /// "file", "date", or None for the running file-header default.
    pub group: Option<String>,
}

/// The heading hierarchy above `line`, root first.
///
/// Headings are collected once per file and walked per task, rather than
/// rescanning the file for every task in it.
fn breadcrumbs_at(headings: &[(usize, usize, String)], line: usize) -> Vec<String> {
    let mut stack: Vec<(usize, String)> = Vec::new();
    for (index, level, text) in headings {
        if *index >= line {
            break;
        }
        stack.retain(|(l, _)| l < level);
        stack.push((*level, text.clone()));
    }
    stack.into_iter().map(|(_, text)| text).collect()
}

fn collect_headings(lines: &[&str]) -> Vec<(usize, usize, String)> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let level = line.bytes().take_while(|b| *b == b'#').count();
            if level == 0 || level > 6 {
                return None;
            }
            let rest = &line[level..];
            let text = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('\t'))?;
            let text = text.trim();
            (!text.is_empty()).then(|| (index, level, text.to_string()))
        })
        .collect()
}

pub fn find_tasks(config: &Config, f: &Filters) -> Result<Vec<Task>> {
    let due_range = match (f.due_from, f.due_to, f.due_on) {
        (None, None, Some(on)) => {
            let d = require_ymd(on, "--due-on")?;
            Some((d, d.succ_opt().unwrap_or(NaiveDate::MAX)))
        }
        _ => inclusive_range(f.due_from, f.due_to, "--due-from", "--due-to")?,
    };
    let created_range = inclusive_range(
        f.created_from,
        f.created_to,
        "--created-from",
        "--created-to",
    )?;
    let done_range = inclusive_range(f.done_from, f.done_to, "--done-from", "--done-to")?;

    let all_statuses = resolve_statuses(config);
    let today = chrono::Local::now().date_naive();

    // Which statuses to keep. --status wins; otherwise the done flags decide.
    let allowed: Option<Vec<String>> = if !f.status.is_empty() {
        if f.status.iter().any(|s| s == "all") {
            None
        } else {
            Some(f.status.to_vec())
        }
    } else if f.only_done {
        Some(vec!["done".into()])
    } else if f.include_done {
        None
    } else {
        Some(
            all_statuses
                .iter()
                .filter(|(_, s)| !s.closed)
                .map(|(n, _)| n.clone())
                .collect(),
        )
    };

    let mut tasks = Vec::new();

    for path in all_notes(config) {
        let relative = relative_path(&config.vault_path, &path);

        if let Some(filter) = f.file {
            if !relative.starts_with(filter) && !relative.contains(filter) {
                continue;
            }
        }
        if f.exclude
            .iter()
            .any(|p| relative.starts_with(p.as_str()) || relative.contains(p.as_str()))
        {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lines: Vec<&str> = content.split('\n').collect();

        let headings = collect_headings(&lines);
        let wants_context = f.context_before > 0 || f.context_after > 0;

        let candidates: Vec<(usize, char, String)> = if is_org(&path) {
            org_candidates(&content, &lines)
        } else {
            let outliner = outliner_candidates(&lines, &config.flavor);
            let seen: Vec<usize> = outliner.iter().map(|(i, _, _)| *i).collect();
            let mut all = outliner;
            for (index, line) in lines.iter().enumerate() {
                if seen.contains(&index) {
                    continue;
                }
                if let Some(c) = CHECKBOX.captures(line) {
                    let ch = c[2].chars().next().unwrap_or(' ');
                    all.push((index, ch, c[3].trim().to_string()));
                }
            }
            all.sort_by_key(|(i, _, _)| *i);
            all
        };

        for (index, checkbox, text) in candidates {
            let name = status_name(checkbox, &all_statuses);
            let closed = all_statuses.get(&name).map(|s| s.closed).unwrap_or(false);

            if let Some(allowed) = &allowed {
                if !allowed.contains(&name) {
                    continue;
                }
            }

            let task = Task {
                file: relative.clone(),
                line: index + 1,
                done: closed && name == "done",
                status: name,
                status_char: checkbox,
                closed,
                due_date: parse_due_date(&text),
                created_date: parse_created_date(&text),
                done_date: parse_done_date(&text),
                scheduled_date: parse_scheduled_date(&text),
                start_date: parse_start_date(&text),
                priority: parse_priority(&text),
                recurrence: parse_recurrence(&text),
                tags: TAG_IN_TASK
                    .captures_iter(&text)
                    .map(|c| c[1].to_string())
                    .collect(),
                breadcrumbs: breadcrumbs_at(&headings, index),
                context: wants_context.then(|| {
                    let start = index.saturating_sub(f.context_before);
                    let end = (index + f.context_after + 1).min(lines.len());
                    lines[start..end].join("\n")
                }),
                text,
            };

            if f.recurring_only && task.recurrence.is_none() {
                continue;
            }
            if f.has_date
                && task.due_date.is_none()
                && task.created_date.is_none()
                && task.done_date.is_none()
            {
                continue;
            }
            if due_range.is_some() && !in_range(&task.due_date, &due_range) {
                continue;
            }
            if created_range.is_some() && !in_range(&task.created_date, &created_range) {
                continue;
            }
            if done_range.is_some() && !in_range(&task.done_date, &done_range) {
                continue;
            }
            if f.overdue {
                match task
                    .due_date
                    .as_deref()
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                {
                    Some(d) if d < today && !task.closed => {}
                    _ => continue,
                }
            }
            if let Some(tag) = f.tag {
                if !task.tags.iter().any(|t| t == tag) {
                    continue;
                }
            }

            tasks.push(task);
        }
    }

    Ok(tasks)
}

pub fn print_tasks(tasks: &[Task], format: &str, display: &Display) {
    match format {
        "json" => {
            let items: Vec<_> = tasks
                .iter()
                .map(|t| {
                    let mut item = serde_json::Map::new();
                    item.insert("file".into(), json!(t.file));
                    item.insert("line".into(), json!(t.line));
                    item.insert("text".into(), json!(t.text));
                    item.insert("done".into(), json!(t.done));
                    item.insert("status".into(), json!(t.status));
                    let optionals: [(&str, &Option<String>); 6] = [
                        ("due_date", &t.due_date),
                        ("created_date", &t.created_date),
                        ("done_date", &t.done_date),
                        ("scheduled_date", &t.scheduled_date),
                        ("start_date", &t.start_date),
                        ("priority", &t.priority),
                    ];
                    for (key, value) in optionals {
                        if let Some(v) = value {
                            item.insert(key.into(), json!(v));
                        }
                    }
                    if let Some((interval, unit)) = &t.recurrence {
                        item.insert(
                            "recurrence".into(),
                            json!({"interval": interval, "unit": unit}),
                        );
                    }
                    if !t.tags.is_empty() {
                        item.insert("tags".into(), json!(t.tags));
                    }
                    serde_json::Value::Object(item)
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&items).unwrap());
        }
        "paths" => {
            let mut files: Vec<&String> = tasks.iter().map(|t| &t.file).collect();
            files.sort();
            files.dedup();
            files.iter().for_each(|f| println!("{f}"));
        }
        _ => {
            if tasks.is_empty() {
                println!("No tasks found.");
                return;
            }
            match display.group.as_deref() {
                Some("file") => {
                    let mut by_file: BTreeMap<&str, Vec<&Task>> = BTreeMap::new();
                    for task in tasks {
                        by_file.entry(&task.file).or_default().push(task);
                    }
                    for (file, group) in by_file {
                        println!("\n{file}:");
                        for task in group {
                            print_task(task, display, false);
                        }
                    }
                }
                Some("date") => {
                    // Undated tasks sort last, where a reader expects them,
                    // rather than first as "(" would put them.
                    let mut by_date: BTreeMap<(bool, &str), Vec<&Task>> = BTreeMap::new();
                    for task in tasks {
                        let key = match task.due_date.as_deref() {
                            Some(d) => (false, d),
                            None => (true, "(no due date)"),
                        };
                        by_date.entry(key).or_default().push(task);
                    }
                    for ((_, date), group) in by_date {
                        println!("\n{date}:");
                        for task in group {
                            print_task(task, display, true);
                        }
                    }
                }
                _ => {
                    let mut current = "";
                    for task in tasks {
                        if task.file != current {
                            println!("\n{}:", task.file);
                            current = &task.file;
                        }
                        print_task(task, display, false);
                    }
                }
            }
            let files: std::collections::BTreeSet<_> = tasks.iter().map(|t| &t.file).collect();
            println!("\nFound {} tasks in {} files", tasks.len(), files.len());
        }
    }
}

fn print_task(task: &Task, display: &Display, show_file: bool) {
    if display.breadcrumbs && !task.breadcrumbs.is_empty() {
        let crumbs = task.breadcrumbs.join(" > ");
        if show_file {
            println!("  {} > {crumbs}:", task.file);
        } else {
            println!("  {crumbs}:");
        }
    } else if show_file {
        println!("  {}:", task.file);
    }

    println!("  {}: - [{}] {}", task.line, task.status_char, task.text);

    if let Some(context) = &task.context {
        for line in context.split('\n') {
            println!("      {line}");
        }
    }
}

// ---------------------------------------------------------------- mutation --

/// Format a date with the moment-style tokens the config uses.
fn format_marker(format: &str, date: NaiveDate) -> String {
    format
        .replace("YYYY", &date.format("%Y").to_string())
        .replace("MM", &date.format("%m").to_string())
        .replace("DD", &date.format("%d").to_string())
}

/// Find the one line a mutation should touch.
///
/// Ambiguity is an error rather than a guess: rewriting the wrong task is
/// worse than refusing, and the caller can narrow with --file.
fn locate_task(
    config: &Config,
    text: &str,
    file_filter: Option<&str>,
    partial: bool,
) -> Result<(std::path::PathBuf, usize, String)> {
    let mut hits = Vec::new();

    for path in all_notes(config) {
        let relative = relative_path(&config.vault_path, &path);
        if let Some(filter) = file_filter {
            if !relative.starts_with(filter) && !relative.contains(filter) {
                continue;
            }
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (index, line) in content.split('\n').enumerate() {
            let Some(c) = CHECKBOX.captures(line) else {
                continue;
            };
            let body = c[3].trim();
            let matched = if partial {
                body.contains(text)
            } else {
                // The stored text carries markers the caller did not type.
                body == text || body.starts_with(&format!("{text} "))
            };
            if matched {
                hits.push((path.clone(), index, relative.clone()));
            }
        }
    }

    match hits.len() {
        0 => Err(anyhow!("No task found matching \"{text}\"")),
        1 => Ok(hits.remove(0)),
        n => Err(anyhow!(
            "Found {n} tasks matching \"{text}\":\n\nNarrow down with --file or use more specific text."
        )),
    }
}

/// Move a task to a named status, appending its date marker if it has one.
pub fn set_status(
    config: &Config,
    status: &str,
    text: &str,
    file_filter: Option<&str>,
    partial: bool,
) -> Result<()> {
    let resolved = resolve_statuses(config);
    let target = resolved
        .get(status)
        .ok_or_else(|| anyhow!("Unknown status: {status}"))?;

    let (path, index, relative) = locate_task(config, text, file_filter, partial)?;
    let content = std::fs::read_to_string(&path)?;
    let mut lines: Vec<String> = content.split('\n').map(str::to_string).collect();

    let line = &lines[index];
    let caps = CHECKBOX
        .captures(line)
        .ok_or_else(|| anyhow!("Line {} is not a task", index + 1))?;
    let indent = caps[1].to_string();
    let mut body = caps[3].trim().to_string();

    // Drop any existing status date before adding the new one, so repeated
    // transitions do not stack markers.
    for other in resolved.values() {
        if let Some(format) = &other.date_format {
            if let Some(marker) = format.split_whitespace().next() {
                let pattern = Regex::new(&format!(
                    r"\s*{}\s*\d{{4}}-\d{{2}}-\d{{2}}",
                    regex::escape(marker)
                ))?;
                body = pattern.replace_all(&body, "").trim().to_string();
            }
        }
    }

    if let Some(format) = &target.date_format {
        body = format!(
            "{body} {}",
            format_marker(format, chrono::Local::now().date_naive())
        );
    }

    lines[index] = format!("{indent}- [{}] {body}", target.char);
    std::fs::write(&path, lines.join("\n"))?;

    // Same line the Python implementation prints, so a caller parsing stdout
    // sees one format regardless of which binary it invoked.
    println!(
        "[{}] {status}: {body} ({relative}:{})",
        target.char,
        index + 1
    );
    Ok(())
}

/// Append a task to the daily note, the inbox, or a named file.
pub fn new_task(
    config: &Config,
    text: &str,
    file: Option<&str>,
    due: Option<&str>,
    recurring: Option<&str>,
    priority: Option<&str>,
) -> Result<()> {
    let target = match file.unwrap_or(&config.tasks_default_file) {
        "daily" => {
            let name = format_marker(&config.daily_format, chrono::Local::now().date_naive());
            format!("{}/{name}.md", config.daily_folder.trim_end_matches('/'))
        }
        "inbox" => config.tasks_inbox.clone(),
        path => path.to_string(),
    };
    let path = config.vault_path.join(&target);

    let mut line = format!("- [ ] {text}");
    if let Some(p) = priority {
        let marker = match p {
            "high" => "⏫",
            "medium" => "🔼",
            _ => "🔽",
        };
        line.push_str(&format!(" {marker}"));
    }
    if config.tasks_created_date {
        line.push_str(&format!(
            " {}",
            format_marker(
                &config.tasks_created_date_format,
                chrono::Local::now().date_naive()
            )
        ));
    }
    if let Some(r) = recurring {
        line.push_str(&format!(" 🔁 {}", r.trim()));
    }
    if let Some(d) = due {
        let date = require_ymd(d, "--due")?;
        line.push_str(&format!(" 📅 {}", date.format("%Y-%m-%d")));
    }

    if path.exists() {
        let mut content = std::fs::read_to_string(&path)?;
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&line);
        content.push('\n');
        std::fs::write(&path, content)?;
        println!("✅ Created task: {target}");
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, format!("# Tasks\n\n{line}\n"))?;
        println!("✅ Created task in new file: {target}");
    }
    println!("   {line}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::StatusOverride;

    fn statuses() -> BTreeMap<String, Status> {
        resolve_statuses(&Config::default())
    }

    #[test]
    fn due_dates_are_read_from_every_supported_marker() {
        assert_eq!(
            parse_due_date("Task 📅 2026-01-26").as_deref(),
            Some("2026-01-26")
        );
        assert_eq!(
            parse_due_date("Task 📅2026-01-26").as_deref(),
            Some("2026-01-26")
        );
        assert_eq!(
            parse_due_date("Task due:2026-01-26").as_deref(),
            Some("2026-01-26")
        );
        assert_eq!(parse_due_date("Task without due date"), None);
    }

    /// The done and created markers were once built with a pattern that
    /// required a literal backslash and so never matched. Tasks still listed,
    /// they just silently never carried these dates.
    #[test]
    fn done_and_created_markers_actually_parse() {
        assert_eq!(
            parse_done_date("- [x] ship it ✅ 2026-07-20").as_deref(),
            Some("2026-07-20")
        );
        assert_eq!(
            parse_created_date("- [ ] ship it ➕ 2026-05-17 📅 2026-05-20").as_deref(),
            Some("2026-05-17")
        );
    }

    #[test]
    fn done_and_created_have_plain_text_forms_too() {
        for line in ["- [x] x done:2026-07-20", "- [x] x @done(2026-07-20)"] {
            assert_eq!(
                parse_done_date(line).as_deref(),
                Some("2026-07-20"),
                "{line}"
            );
        }
        for line in ["- [ ] x created:2026-07-01", "- [ ] x @created(2026-07-01)"] {
            assert_eq!(
                parse_created_date(line).as_deref(),
                Some("2026-07-01"),
                "{line}"
            );
        }
    }

    #[test]
    fn scheduled_start_and_priority_markers_parse() {
        assert_eq!(
            parse_scheduled_date("x ⏳ 2026-02-01").as_deref(),
            Some("2026-02-01")
        );
        assert_eq!(
            parse_start_date("x 🛫 2026-02-01").as_deref(),
            Some("2026-02-01")
        );
        assert_eq!(parse_priority("x ⏫").as_deref(), Some("high"));
        assert_eq!(parse_priority("x 🔼").as_deref(), Some("medium"));
        assert_eq!(parse_priority("x 🔽").as_deref(), Some("low"));
        assert_eq!(parse_priority("x"), None);
    }

    #[test]
    fn recurrence_is_read_as_an_interval_and_unit() {
        assert_eq!(
            parse_recurrence("Task 🔁 every 6 months 📅 2026-02-03"),
            Some((6, "months".to_string()))
        );
        assert_eq!(
            parse_recurrence("Task 🔁 every 1 year"),
            Some((1, "years".to_string()))
        );
        assert_eq!(parse_recurrence("Task without recurrence"), None);
    }

    /// Date filters must reject what they cannot parse. They used to fall
    /// back to an unbounded range, which silently returned every task
    /// instead of reporting the bad input.
    #[test]
    fn date_filters_reject_relative_and_malformed_values() {
        for bad in [
            "today",
            "+3d",
            "tomorrow",
            "this week",
            "2026-13-01",
            "not-a-date",
        ] {
            let err = require_ymd(bad, "--due-on").unwrap_err().to_string();
            assert!(err.contains("--due-on"), "{bad}: {err}");
            assert!(err.contains("YYYY-MM-DD"), "{bad}: {err}");
        }
        assert!(require_ymd("2026-01-26", "--due-on").is_ok());
    }

    #[test]
    fn the_built_in_statuses_are_open_done_wip_and_cancel() {
        let s = statuses();
        assert_eq!(s["open"].char, ' ');
        assert_eq!(s["done"].char, 'x');
        assert_eq!(s["wip"].char, '/');
        assert_eq!(s["cancel"].char, '-');
        assert!(s["done"].closed);
        assert!(s["cancel"].closed);
        assert!(!s["wip"].closed);
        assert!(!s["open"].closed);
    }

    #[test]
    fn a_checkbox_character_maps_to_its_status() {
        let s = statuses();
        assert_eq!(status_name(' ', &s), "open");
        assert_eq!(status_name('x', &s), "done");
        assert_eq!(status_name('X', &s), "done");
        assert_eq!(status_name('/', &s), "wip");
        assert_eq!(status_name('-', &s), "cancel");
        // An unconfigured character is still a task, just an unnamed one.
        assert_eq!(status_name('>', &s), "unknown");
    }

    #[test]
    fn the_config_can_add_a_status_and_override_a_built_in_one() {
        let mut config = Config::default();
        config.tasks_statuses.insert(
            "forward".into(),
            StatusOverride {
                char: Some('>'),
                closed: Some(true),
                date_format: Some("➡️ YYYY-MM-DD".into()),
                date_format_set: true,
            },
        );
        config.tasks_statuses.insert(
            "cancel".into(),
            StatusOverride {
                char: None,
                closed: None,
                date_format: None,
                date_format_set: true,
            },
        );

        let s = resolve_statuses(&config);
        assert_eq!(s["forward"].char, '>');
        assert!(s["forward"].closed);
        assert_eq!(s["forward"].date_format.as_deref(), Some("➡️ YYYY-MM-DD"));
        // An explicit null clears the default ❌ marker.
        assert_eq!(s["cancel"].date_format, None);
        assert_eq!(status_name('>', &s), "forward");
    }

    #[test]
    fn disabling_the_done_date_removes_its_marker() {
        let config = Config {
            tasks_done_date: false,
            ..Default::default()
        };
        assert_eq!(resolve_statuses(&config)["done"].date_format, None);
    }

    #[test]
    fn breadcrumbs_are_the_heading_path_above_a_line() {
        let lines = vec![
            "# Top Level",
            "Some text",
            "## Second Level",
            "More text",
            "### Third Level",
            "- [ ] Task here",
        ];
        let headings = collect_headings(&lines);
        assert_eq!(
            breadcrumbs_at(&headings, 5),
            ["Top Level", "Second Level", "Third Level"]
        );
    }

    /// A deeper heading is replaced, not stacked, once a shallower one
    /// follows it.
    #[test]
    fn a_shallower_heading_pops_the_deeper_ones() {
        let lines = vec!["# A", "## B", "### C", "## D", "- [ ] t"];
        let headings = collect_headings(&lines);
        assert_eq!(breadcrumbs_at(&headings, 4), ["A", "D"]);
    }

    #[test]
    fn a_hash_without_a_space_is_not_a_heading() {
        assert!(collect_headings(&["#tag", "####### too deep"]).is_empty());
    }
}
