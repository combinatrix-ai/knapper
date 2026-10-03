//! Dataview's pinned DQL engine, embedded in a capability-free QuickJS runtime.
use anyhow::{anyhow, Context as _, Result};
use rquickjs::{Context, Function, Promise, Runtime};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const ENGINE: &str = include_str!("../dql/engine.js");
pub const NOTICES: &str = include_str!("../dql/THIRD_PARTY_NOTICES.txt");

pub(crate) fn js_error(ctx: &rquickjs::Ctx<'_>, error: rquickjs::Error) -> anyhow::Error {
    if error.is_exception() {
        let value = ctx.catch();
        if let Some(exception) = value.as_exception() {
            return anyhow!(
                "JavaScript: {}\n{}",
                exception.message().unwrap_or_default(),
                exception.stack().unwrap_or_default()
            );
        }
        return anyhow!("JavaScript exception: {value:?}");
    }
    anyhow!("JavaScript: {error}")
}

fn runtime(memory_limit_mib: u32, timeout_seconds: u64) -> Result<(Runtime, Context)> {
    let rt = Runtime::new()?;
    rt.set_memory_limit(memory_limit_mib as usize * 1024 * 1024);
    rt.set_max_stack_size(8 * 1024 * 1024);
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    // No filesystem, network, module loader, process or host callbacks.
    let ctx = Context::full(&rt)?;
    Ok((rt, ctx))
}

pub fn evaluate(input: &Value, memory_limit_mib: u32, timeout_seconds: u64) -> Result<Value> {
    let (_rt, context) = runtime(memory_limit_mib, timeout_seconds)?;
    context.with(|ctx| {
        ctx.eval::<(), _>(ENGINE).map_err(|e| js_error(&ctx, e))?;
        let function: Function = ctx.globals().get("knapperDql")?;
        let promise: Promise = function
            .call((serde_json::to_string(input)?,))
            .map_err(|e| js_error(&ctx, e))?;
        let output: String = promise.finish().map_err(|e| js_error(&ctx, e))?;
        let output: Value = serde_json::from_str(&output)?;
        if let Some(error) = output.get("error").and_then(Value::as_str) {
            return Err(anyhow!("{error}"));
        }
        Ok(output)
    })
}

/// Build the Obsidian-shaped metadata consumed by Dataview's own importer.
/// CommonMark handles sections/list nesting; knapper's scanner handles local
/// wikilinks and its prose mask keeps examples/comments out of the index.
fn markdown_metadata(contents: &str) -> Result<Value> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let (frontmatter, body) = crate::note::split_frontmatter(contents);
    let body_offset = body.as_ptr() as usize - contents.as_ptr() as usize;
    let body_line = contents[..body_offset]
        .bytes()
        .filter(|b| *b == b'\n')
        .count();
    let masked = crate::parser::mask_noncontent(body);
    let lines: Vec<&str> = contents.split('\n').collect();
    let position = |start: usize, end: usize| {
        let start_line = body_line + crate::links::line_of(body, start) - 1;
        let end_line =
            body_line + crate::links::line_of(body, end.saturating_sub(1).max(start)) - 1;
        json!({"start":{"line":start_line,"col":0,"offset":body_offset+start},
               "end":{"line":end_line,"col":lines.get(end_line).map_or(0, |l| l.chars().count()),"offset":body_offset+end}})
    };
    let mut sections = Vec::new();
    let mut headings = Vec::new();
    let mut lists = Vec::<Value>::new();
    let mut stack = Vec::<usize>::new();
    let mut heading: Option<(usize, usize, u8, String)> = None;
    let task_marker = regex::Regex::new(r"^[\s>]*(?:\d+[.)]|[-*+])\s+\[([^\]]?)\]")?;
    let block_id = regex::Regex::new(r"\s\^([A-Za-z0-9-]+)\s*$")?;
    for (event, range) in
        Parser::new_ext(&masked, Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS)
            .into_offset_iter()
    {
        match event {
            Event::Start(Tag::Paragraph) => sections
                .push(json!({"type":"paragraph","position":position(range.start,range.end)})),
            Event::Start(Tag::Heading { level, .. }) => {
                sections.push(json!({"type":"heading","position":position(range.start,range.end)}));
                heading = Some((range.start, range.end, level as u8, String::new()));
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, _, _, title)) = &mut heading {
                    title.push_str(&text);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((start, end, level, title)) = heading.take() {
                    headings.push(
                        json!({"heading":title,"level":level,"position":position(start,end)}),
                    );
                }
            }
            Event::Start(Tag::List(_)) => {
                sections.push(json!({"type":"list","position":position(range.start,range.end)}))
            }
            Event::Start(Tag::Item) => {
                let start_line = body_line + crate::links::line_of(body, range.start) - 1;
                let parent = stack
                    .last()
                    .map(|i| lists[*i]["position"]["start"]["line"].as_i64().unwrap())
                    .unwrap_or(-1);
                // Obsidian's list item range contains its own lines, excluding
                // nested items (unlike CommonMark's enclosing item range).
                if let Some(parent_index) = stack.last() {
                    let first_child = !lists[*parent_index]
                        .get("hasChild")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if first_child {
                        lists[*parent_index]["position"]["end"] = json!({"line":start_line.saturating_sub(1),"col":lines.get(start_line.saturating_sub(1)).map_or(0,|l|l.chars().count()),"offset":body_offset + range.start});
                        lists[*parent_index]["hasChild"] = json!(true);
                    }
                }
                // CommonMark includes trailing blank lines in item ranges;
                // Obsidian's metadata ends on the last content line. Keeping
                // those blanks changes Dataview task text and lineCount.
                let item_end = body[..range.end].trim_end().len().max(range.start);
                let mut item = json!({"position":position(range.start,item_end),"parent":parent});
                if let Some(captures) = lines.get(start_line).and_then(|l| task_marker.captures(l))
                {
                    item["task"] =
                        json!(captures.get(1).map_or(" ", |m| if m.as_str().is_empty() {
                            " "
                        } else {
                            m.as_str()
                        }));
                }
                if let Some(captures) = lines.get(start_line).and_then(|l| block_id.captures(l)) {
                    item["id"] = json!(&captures[1]);
                }
                stack.push(lists.len());
                lists.push(item);
            }
            Event::End(TagEnd::Item) => {
                stack.pop();
            }
            _ => {}
        }
    }
    let mut links = Vec::new();
    let mut embeds = Vec::new();
    let mut frontmatter_links = Vec::new();
    for raw in crate::links::scan_links(contents) {
        let display = if matches!(raw.kind, crate::links::Kind::Wiki) {
            raw.alias.trim_start_matches(['\\', '|']).to_string()
        } else {
            raw.label
        };
        let anchor = if raw.anchor.starts_with('^') {
            format!("#{}", raw.anchor)
        } else {
            raw.anchor
        };
        let link = json!({"link":format!("{}{}",raw.path,anchor),"displayText":display,
                          "position":{"start":{"line":crate::links::line_of(contents,raw.range.start)-1,"col":0,"offset":raw.range.start},
                                      "end":{"line":crate::links::line_of(contents,raw.range.end)-1,"col":0,"offset":raw.range.end}}});
        if raw.range.start < body_offset {
            frontmatter_links.push(link);
        } else if raw.embed {
            embeds.push(link);
        } else {
            links.push(link);
        }
    }
    let tags: Vec<Value> = crate::parser::extract_tags(&masked)
        .into_iter()
        .map(|tag| json!({"tag":format!("#{tag}")}))
        .collect();
    Ok(
        json!({"frontmatter":frontmatter,"frontmatterLinks":frontmatter_links,
              "sections":sections,"headings":headings,"listItems":lists,"tags":tags,"links":links,"embeds":embeds}),
    )
}

fn snapshot(
    config: &crate::vault::Config,
    origin: Option<&str>,
    query: &str,
    timezone: &str,
    obsidian_config_dir: &str,
) -> Result<Value> {
    use std::time::UNIX_EPOCH;
    let bookmarked_files =
        crate::obsidian::bookmarked_files(&config.vault_path, obsidian_config_dir)?;
    let mut files = Vec::new();
    let note_paths: std::collections::HashSet<_> =
        crate::vault::all_notes(config).into_iter().collect();
    for path in crate::vault::all_files(config) {
        let relative = crate::vault::relative_path(&config.vault_path, &path);
        if crate::vault::is_excluded(&relative, &config.exclude) {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let markdown = note_paths.contains(&path)
            && matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown");
        let csv = ext.eq_ignore_ascii_case("csv");
        let mut file = json!({"path":relative});
        if markdown || csv {
            let contents = std::fs::read_to_string(&path)
                .with_context(|| format!("Reading {}", path.display()))?;
            let metadata = std::fs::metadata(&path)?;
            let millis = |time: std::time::SystemTime| {
                time.duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
            };
            file["stat"] = json!({"ctime":millis(metadata.created().unwrap_or_else(|_| metadata.modified().unwrap_or(UNIX_EPOCH))),
                                  "mtime":millis(metadata.modified().unwrap_or(UNIX_EPOCH)),"size":metadata.len()});
            if markdown {
                file["metadata"] = markdown_metadata(&contents)?;
            }
            file["contents"] = json!(contents);
        }
        files.push(file);
    }
    let origin = origin
        .map(|p| {
            crate::vault::relative_path(
                &config.vault_path,
                &crate::vault::resolve_path(&config.vault_path, p),
            )
        })
        .unwrap_or_default();
    if !origin.is_empty()
        && !files
            .iter()
            .any(|f| f["path"] == origin && f.get("metadata").is_some())
    {
        return Err(anyhow!("Origin must be an indexed Markdown note: {origin}"));
    }
    Ok(
        json!({"files":files,"origin":origin,"query":query,"timezone":timezone,"bookmarkedFiles":bookmarked_files}),
    )
}

fn render(value: &Value) -> String {
    match value {
        Value::Null => "-".into(),
        Value::String(s) => s.clone(),
        Value::Array(values) => values.iter().map(render).collect::<Vec<_>>().join(", "),
        Value::Object(object) => match object.get("type").and_then(Value::as_str) {
            Some("date") => object["value"].as_str().unwrap_or_default().to_string(),
            Some("link") => {
                let path = object["path"].as_str().unwrap_or_default();
                let sub = object["subpath"]
                    .as_str()
                    .map(|s| {
                        format!(
                            "#{}{}",
                            if object["linkType"] == "block" {
                                "^"
                            } else {
                                ""
                            },
                            s
                        )
                    })
                    .unwrap_or_default();
                let display = object["display"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("|{s}"))
                    .unwrap_or_default();
                format!("[[{path}{sub}{display}]]")
            }
            _ => value.to_string(),
        },
        _ => value.to_string(),
    }
}

pub struct Options<'a> {
    pub origin: Option<&'a str>,
    pub timezone: Option<&'a str>,
    pub memory_limit_mib: u32,
    pub timeout_seconds: u64,
    pub obsidian_config_dir: &'a str,
    pub format: &'a str,
}

pub fn run(config: &crate::vault::Config, query: &str, options: Options<'_>) -> Result<()> {
    let Options {
        origin,
        timezone,
        memory_limit_mib,
        timeout_seconds,
        obsidian_config_dir,
        format,
    } = options;
    let timezone = timezone
        .map(str::to_string)
        .unwrap_or_else(|| iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into()));
    let output = evaluate(
        &snapshot(config, origin, query, &timezone, obsidian_config_dir)?,
        memory_limit_mib,
        timeout_seconds,
    )?;
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if output["type"] == "table" {
        let headers = output["headers"].as_array().unwrap();
        println!(
            "| {} |",
            headers.iter().map(render).collect::<Vec<_>>().join(" | ")
        );
        println!(
            "| {} |",
            headers
                .iter()
                .map(|_| "---")
                .collect::<Vec<_>>()
                .join(" | ")
        );
        for row in output["values"].as_array().unwrap() {
            println!(
                "| {} |",
                row.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| render(v).replace('|', "\\|").replace('\n', "<br>"))
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
        }
    } else if output["type"] == "list" || output["type"] == "calendar" {
        for value in output["values"].as_array().unwrap() {
            println!("- {}", render(value));
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&output["tasks"])?);
    }
    if let Some(ops) = output["diagnostics"].as_array() {
        let errors: usize = ops
            .iter()
            .filter_map(|op| op["errors"].as_array())
            .map(Vec::len)
            .sum();
        if errors > 0 {
            eprintln!("DQL: {errors} row evaluation error(s); see JSON diagnostics");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unmodified_upstream_suites_run_inside_quickjs() {
        let (_rt, context) = runtime(256, 30).unwrap();
        context.with(|ctx| {
            ctx.eval::<(), _>(include_str!("../dql/upstream-tests.js"))
                .map_err(|e| js_error(&ctx, e))
                .unwrap();
            let report: String = ctx
                .eval("knapperTestReport()")
                .map_err(|e| js_error(&ctx, e))
                .unwrap();
            let report: Value = serde_json::from_str(&report).unwrap();
            assert_eq!(report["total"], 395);
            assert_eq!(report["passed"], report["total"], "{report}");
        });
    }

    #[test]
    #[ignore = "requires Node and the generated native-Intl reference bundle"]
    fn node_reference_matches_embedded_runtime() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let vault = tempfile::tempdir().unwrap();
        std::fs::create_dir(vault.path().join("Diary")).unwrap();
        std::fs::write(vault.path().join("knapper.yaml"), "vault_path: .\n").unwrap();
        std::fs::write(vault.path().join("Diary/2026-10-02.md"), "---\ntags: [mail/history]\nstatus: open\nrelated: '[[2026-10-03]]'\n---\n# Mail\nmessages:: 800\n[accounts:: [[Personal]], [[Work]]]\n- [ ] respond [cost:: 2]\n  - [x] inspect\n").unwrap();
        std::fs::write(
            vault.path().join("Diary/2026-10-03.md"),
            "---\ntags: [mail/history]\nstatus: open\n---\nmessages:: 831\n[accounts:: [[Work]]]\n",
        )
        .unwrap();
        std::fs::write(
            vault.path().join("Diary/counts.csv"),
            "date,count\n2026-10-02,800\n2026-10-03,831\n",
        )
        .unwrap();
        let config = crate::vault::load_config(
            Some(vault.path().join("knapper.yaml").to_str().unwrap()),
            Some(vault.path().to_str().unwrap()),
        )
        .unwrap();
        let queries = [
            "TABLE file.day, messages FROM \"Diary\" SORT file.day",
            "TABLE sum(rows.messages), average(rows.messages) FROM #MAIL GROUP BY status",
            "TABLE meta(account).path, sum(rows.messages) FROM #mail FLATTEN accounts AS account GROUP BY account",
            "LIST WITHOUT ID file.name FROM [[Diary/2026-10-03]]",
            "TABLE this.messages, related.messages FROM \"Diary/2026-10-02\"",
            "TASK FROM #mail WHERE !completed",
            "CALENDAR file.day FROM \"Diary\" SORT file.day",
            "TABLE sum(rows.count) FROM csv(\"counts.csv\") GROUP BY true",
            "TABLE dateformat(file.day, \"DDDD\"), currencyformat(messages, \"JPY\") FROM \"Diary\"",
            "TABLE dateformat(date(\"2026-07-01T12:00:00[America/Toronto]\"), \"ZZ\"), dateformat(date(\"2026-01-01T12:00:00[America/Toronto]\"), \"ZZ\") FROM \"Diary\"",
        ];
        for query in queries {
            let input = snapshot(
                &config,
                Some("Diary/2026-10-02.md"),
                query,
                "Asia/Tokyo",
                ".obsidian",
            )
            .unwrap();
            let embedded = evaluate(&input, 256, 30).unwrap();
            let mut child =
                Command::new(std::env::var("KNAPPER_DQL_NODE").unwrap_or_else(|_| "node".into()))
                    .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/dql/reference.mjs"))
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(serde_json::to_string(&input).unwrap().as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let reference: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(embedded, reference, "{query}");
        }
    }
}
