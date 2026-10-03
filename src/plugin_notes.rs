//! Explicit, single-note community plugin operations.
use crate::vault::{relative_path, Config};
use anyhow::{anyhow, Result};
use serde_json::json;
use std::path::PathBuf;

fn note_path(config: &Config, file: &str) -> Result<PathBuf> {
    let vault = config.vault_path.canonicalize()?;
    let path = vault.join(file).canonicalize()?;
    let relative = relative_path(&vault, &path);
    if crate::vault::is_excluded(&relative, &config.exclude)
        || relative.split('/').any(|part| part.starts_with('.'))
    {
        return Err(anyhow!("Note is outside the configured note scope"));
    }
    if !path.starts_with(&vault)
        || !path.is_file()
        || !matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("md" | "markdown")
        )
    {
        return Err(anyhow!("Expected a Markdown file inside the vault"));
    }
    if !crate::vault::all_notes(config)
        .iter()
        .any(|p| p.canonicalize().ok().as_ref() == Some(&path))
    {
        return Err(anyhow!("Note is outside the configured note scope"));
    }
    Ok(path)
}

pub fn format_note(
    config: &Config,
    file: &str,
    rules: &[String],
    apply: bool,
    format: &str,
) -> Result<()> {
    let path = note_path(config, file)?;
    let before = std::fs::read_to_string(&path)?;
    // Match the selected upstream rule order; trailing spaces runs last.
    let ordered: Vec<_> = [
        "remove-multiple-spaces",
        "heading-blank-lines",
        "trailing-spaces",
    ]
    .into_iter()
    .filter(|r| rules.iter().any(|v| v == r))
    .collect();
    let result = crate::tasks_query::evaluate_engine(
        include_str!("../plugins/linter-engine.js"),
        "knapperLinter",
        &json!({"text":before,"rules":ordered}),
    )?;
    let after = result["text"]
        .as_str()
        .ok_or_else(|| anyhow!("Linter returned no text"))?;
    if apply && after != before {
        if std::fs::read_to_string(&path)? != before {
            return Err(anyhow!("Note changed during formatting; rerun preview"));
        }
        std::fs::write(&path, after)?;
    }
    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"path":relative_path(&config.vault_path,&path),"rules":ordered,"changed":after!=before,"applied":apply && after!=before,"before":before,"after":after})
            )?
        );
    } else {
        println!(
            "{} {}",
            if apply { "Applied:" } else { "Preview:" },
            relative_path(&config.vault_path, &path)
        );
        print!("{after}");
        if !after.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

pub struct CaptureOptions<'a> {
    pub text: &'a str,
    pub template: Option<&'a str>,
    pub position: &'a str,
    pub create: bool,
    pub apply: bool,
    pub format: &'a str,
}

pub fn capture(config: &Config, file: &str, options: &CaptureOptions<'_>) -> Result<()> {
    let candidate = config.vault_path.join(file);
    let path = if candidate.exists() {
        note_path(config, file)?
    } else {
        if !options.create {
            return Err(anyhow!(
                "Capture target does not exist; use --create explicitly"
            ));
        }
        let vault = config.vault_path.canonicalize()?;
        let parent = candidate
            .parent()
            .ok_or_else(|| anyhow!("Invalid capture target"))?
            .canonicalize()?;
        let path = parent.join(
            candidate
                .file_name()
                .ok_or_else(|| anyhow!("Invalid capture filename"))?,
        );
        let relative = relative_path(&vault, &path);
        if !path.starts_with(&vault)
            || !matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("md" | "markdown")
            )
            || relative.split('/').any(|p| p.starts_with('.'))
            || crate::vault::is_excluded(&relative, &config.exclude)
        {
            return Err(anyhow!(
                "Capture target is outside the configured Markdown scope"
            ));
        }
        path
    };
    let existed = path.exists();
    let before = if existed {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let template = options
        .template
        .map(|file| {
            note_path(config, file)
                .and_then(|path| std::fs::read_to_string(path).map_err(anyhow::Error::from))
        })
        .transpose()?;
    let result = crate::tasks_query::evaluate_engine(
        include_str!("../plugins/quickadd-engine.js"),
        "knapperCapture",
        &json!({"before":before,"text":options.text,"template":template,"position":options.position}),
    )?;
    let after = result["text"]
        .as_str()
        .ok_or_else(|| anyhow!("Capture returned no text"))?;
    let changed = after != before;
    if options.apply && changed {
        if existed {
            if std::fs::read_to_string(&path)? != before {
                return Err(anyhow!("Note changed during capture; rerun preview"));
            }
            std::fs::write(&path, after)?;
        } else {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?
                .write_all(after.as_bytes())?;
        }
    }
    let relative = relative_path(&config.vault_path.canonicalize()?, &path);
    if options.format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"path":relative,"changed":changed,"applied":options.apply && changed,"before":before,"after":after,"cursor":result["cursor"]})
            )?
        );
    } else {
        println!(
            "{} {relative}",
            if options.apply {
                "Applied:"
            } else {
                "Preview:"
            }
        );
        print!("{after}");
        if !after.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}
