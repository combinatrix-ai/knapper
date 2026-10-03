//! Pinned Obsidian Tasks query engine; separate from native task mutations.
use crate::vault::{all_notes, relative_path, Config};
use anyhow::{anyhow, Result};
use rquickjs::{Context, Function, Promise, Runtime};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
const ENGINE: &str = include_str!("../plugins/tasks-engine.js");

pub fn evaluate(input: &Value) -> Result<Value> {
    evaluate_engine(ENGINE, "knapperTasks", input)
}

pub(crate) fn evaluate_engine(engine: &str, name: &str, input: &Value) -> Result<Value> {
    let runtime = Runtime::new()?;
    runtime.set_memory_limit(256 * 1024 * 1024);
    runtime.set_max_stack_size(8 * 1024 * 1024);
    let deadline = Instant::now() + Duration::from_secs(30);
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let context = Context::full(&runtime)?;
    context.with(|ctx| {
        ctx.eval::<(), _>(engine)
            .map_err(|e| crate::dql::js_error(&ctx, e))?;
        let function: Function = ctx.globals().get(name)?;
        let promise: Promise = function
            .call((serde_json::to_string(input)?,))
            .map_err(|e| crate::dql::js_error(&ctx, e))?;
        let output: String = promise
            .finish()
            .map_err(|e| crate::dql::js_error(&ctx, e))?;
        let value: Value = serde_json::from_str(&output)?;
        if let Some(error) = value.get("error").and_then(Value::as_str) {
            return Err(anyhow!("{error}"));
        }
        Ok(value)
    })
}

pub fn query(config: &Config, query: &str, format: &str) -> Result<()> {
    if !matches!(format, "json" | "text") {
        return Err(anyhow!("Tasks query format must be text or json"));
    }
    let mut lines = Vec::new();
    for file in all_notes(config) {
        if !matches!(
            file.extension().and_then(|s| s.to_str()),
            Some("md" | "markdown")
        ) {
            continue;
        }
        let contents = std::fs::read_to_string(&file)?;
        let (_, body) = crate::note::split_frontmatter(&contents);
        let offset = contents[..contents.len() - body.len()]
            .bytes()
            .filter(|b| *b == b'\n')
            .count();
        let masked = crate::parser::mask_noncontent(body);
        let original: Vec<&str> = body.lines().collect();
        let mut heading = None;
        for (index, line) in masked.lines().enumerate() {
            if let Some(title) = line.trim_start().strip_prefix('#').and_then(|s| {
                let s = s.trim_start_matches('#');
                s.strip_prefix(' ')
            }) {
                heading = Some(title.to_string());
            }
            if line.trim().is_empty() {
                continue;
            }
            lines.push(json!({"path":relative_path(&config.vault_path,&file),"line":index+offset,"text":original[index],"heading":heading}));
        }
    }
    let result =
        evaluate(&json!({"query":query,"lines":lines,"now":chrono::Utc::now().to_rfc3339()}))?;
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for group in result["groups"].as_array().unwrap() {
            for name in group["names"].as_array().unwrap() {
                println!("#### {}", name.as_str().unwrap_or_default());
            }
            for task in group["tasks"].as_array().unwrap() {
                println!(
                    "{} ({}:{})",
                    task["markdown"].as_str().unwrap(),
                    task["path"].as_str().unwrap(),
                    task["line"]
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_upstream_suites_run_in_quickjs() {
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            ctx.eval::<(), _>(include_str!("../plugins/upstream-tests.js"))
                .unwrap();
            let f: Function = ctx.globals().get("knapperTestReport").unwrap();
            let raw: String = f.call(()).unwrap();
            let result: Value = serde_json::from_str(&raw).unwrap();
            assert_eq!(result["total"], result["passed"], "{result}");
            assert_eq!(result["total"], 134, "{result}");
        });
    }
    #[test]
    fn rejects_custom_functions_and_unsupported_host_syntax() {
        for query in [
            "filter by function (() => 1)()",
            "preset unsafe",
            "due today",
            "description includes {{query.file.path}}",
        ] {
            assert!(
                evaluate(&json!({"query":query,"lines":[]})).is_err(),
                "{query}"
            );
        }
    }
}
