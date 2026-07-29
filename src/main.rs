//! knapper - a headless CLI for markdown knowledge bases.
//!
//! Reads and writes note files directly; nothing needs to be running.
//! Behaviour is pinned by tests/contract/cases.yaml, which this binary and the
//! Python implementation both answer to.

// The port is partial: several struct fields mirror the Python shape and are
// read by commands that have not landed yet. Removing them now would mean
// adding them back, and the two implementations should stay comparable.
#![allow(dead_code)]

mod commands;
mod graph;
mod note;
mod notes_cmd;
mod org;
mod parser;
mod query;
mod tasks;
mod templater;
mod update;
mod vault;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "knapper",
    version,
    about = "A headless CLI for markdown knowledge bases",
    long_about = "knapper - a headless CLI for markdown knowledge bases.\n\n\
                  Reads and writes markdown files directly; nothing needs to be running.\n\
                  Resolves both [[wikilinks]] and [inline](links.md)."
)]
struct Cli {
    /// Path to config file
    #[arg(short = 'c', long = "config", global = true)]
    config: Option<String>,

    /// Path to vault (overrides config)
    #[arg(short = 'v', long = "vault", global = true)]
    vault: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum FrontmatterCommand {
    /// Get frontmatter value(s).
    Get {
        file: String,
        key: Option<String>,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Set frontmatter value.
    Set {
        file: String,
        key: String,
        value: String,
    },
    /// Delete frontmatter key.
    Delete { file: String, key: String },
}

#[derive(Subcommand)]
enum TaskCommand {
    /// Create a new task.
    New {
        text: String,
        #[arg(long = "file", help = "Target file (overrides config default)")]
        file: Option<String>,
        #[arg(long = "due", help = "Due date (YYYY-MM-DD)")]
        due: Option<String>,
        #[arg(long = "recurring", help = "Recurrence (e.g. 'every 3 months')")]
        recurring: Option<String>,
        #[arg(long = "priority", value_parser = ["high", "medium", "low"])]
        priority: Option<String>,
    },
    /// Mark a task as done.
    Done {
        text: String,
        #[arg(long = "file")]
        file: Option<String>,
        #[arg(long = "partial", help = "Use partial matching (contains)")]
        partial: bool,
    },
    /// Mark a task as in progress.
    Wip {
        text: String,
        #[arg(long = "file")]
        file: Option<String>,
        #[arg(long = "partial")]
        partial: bool,
    },
    /// Mark a task as cancelled.
    Cancel {
        text: String,
        #[arg(long = "file")]
        file: Option<String>,
        #[arg(long = "partial")]
        partial: bool,
    },
    /// Transition a task to a named status.
    Set {
        status: String,
        text: String,
        #[arg(long = "file")]
        file: Option<String>,
        #[arg(long = "partial")]
        partial: bool,
    },
}

// A clap command enum is built once, at startup, from argv. The size of its
// largest variant costs nothing here, and splitting the flags of one command
// into a boxed struct would only obscure what the CLI accepts.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Command {
    /// List outgoing links from a file.
    Links {
        file: String,
        #[arg(
            short = 'A',
            long = "after",
            default_value_t = 0,
            help = "Lines after match"
        )]
        after: usize,
        #[arg(
            short = 'B',
            long = "before",
            default_value_t = 0,
            help = "Lines before match"
        )]
        before: usize,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// List incoming links to a file.
    Backlinks {
        file: String,
        #[arg(
            short = 'A',
            long = "after",
            default_value_t = 0,
            help = "Lines after match"
        )]
        after: usize,
        #[arg(
            short = 'B',
            long = "before",
            default_value_t = 0,
            help = "Lines before match"
        )]
        before: usize,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Find orphan notes (not linked by any other note).
    Orphans {
        #[arg(
            long = "include-special",
            help = "Include Templates/ and hidden folders"
        )]
        include_special: bool,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Find hub notes (most linked-to notes).
    Hubs {
        #[arg(short = 'l', long = "limit", default_value_t = 10)]
        limit: usize,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Find broken links (links to non-existent notes).
    #[command(name = "broken-links")]
    BrokenLinks {
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Initialize a new knapper config file.
    Init {
        #[arg(short = 'f', long = "force")]
        force: bool,
    },
    /// Replace this binary with the newest release.
    SelfUpdate {
        /// Report whether a newer release exists, without installing it.
        #[arg(long = "check")]
        check: bool,
        /// Skip the confirmation prompt.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },
    /// Get comprehensive context for a file (for LLMs).
    Context {
        file: String,
        #[arg(long = "no-content", help = "Exclude file content")]
        no_content: bool,
        #[arg(long = "no-backlinks", help = "Exclude backlinks (faster)")]
        no_backlinks: bool,
        #[arg(long = "no-tasks", help = "Exclude tasks")]
        no_tasks: bool,
        #[arg(long = "max-content", help = "Truncate content to N characters")]
        max_content: Option<usize>,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Read and write frontmatter.
    #[command(subcommand)]
    Frontmatter(FrontmatterCommand),
    /// Check vault health.
    Lint {
        #[arg(long = "check")]
        check: Vec<String>,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Create or get daily note.
    Daily {
        date: Option<String>,
        #[arg(short = 'p', long = "path-only")]
        path_only: bool,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Rename a note and update all links to it.
    Rename {
        old: String,
        new: String,
        #[arg(long = "dry-run")]
        dry_run: bool,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Filter notes by frontmatter, inline fields, and link counts.
    Query {
        #[arg(
            short = 'w',
            long = "where",
            help = "Filter, e.g. status=open, inlinks>3, tags~x"
        )]
        r#where: Vec<String>,
        #[arg(long = "field", help = "Extra column to show (repeatable)")]
        field: Vec<String>,
        #[arg(long = "from", help = "Only notes under this path")]
        from: Option<String>,
        #[arg(long = "sort", help = "Sort by a field, e.g. inlinks:desc")]
        sort: Option<String>,
        #[arg(short = 'l', long = "limit")]
        limit: Option<usize>,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// List what query can filter on, computed and declared.
    Fields {
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Move a note to a different folder and update all links.
    Move {
        source: String,
        destination: String,
        #[arg(long = "dry-run")]
        dry_run: bool,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Find and filter tasks (- [ ] items) in the vault.
    Tasks {
        #[command(subcommand)]
        action: Option<TaskCommand>,
        #[arg(long = "all", help = "Include completed tasks")]
        all: bool,
        #[arg(long = "done", help = "Show only completed tasks")]
        done: bool,
        #[arg(long = "recurring", help = "Show only recurring tasks")]
        recurring: bool,
        #[arg(long = "overdue", help = "Show only overdue tasks")]
        overdue: bool,
        #[arg(long = "has-date", help = "Show only tasks with any date marker")]
        has_date: bool,
        #[arg(long = "due-on")]
        due_on: Option<String>,
        #[arg(long = "due-from")]
        due_from: Option<String>,
        #[arg(long = "due-to")]
        due_to: Option<String>,
        #[arg(long = "created-from")]
        created_from: Option<String>,
        #[arg(long = "created-to")]
        created_to: Option<String>,
        #[arg(long = "done-from")]
        done_from: Option<String>,
        #[arg(long = "done-to")]
        done_to: Option<String>,
        #[arg(long = "file", help = "Filter by file/folder path")]
        file: Option<String>,
        #[arg(long = "exclude", help = "Exclude files/folders (repeatable)")]
        exclude: Vec<String>,
        #[arg(long = "tag", help = "Filter by tag")]
        tag: Option<String>,
        #[arg(long = "status", help = "Filter by status name (repeatable)")]
        status: Vec<String>,
        #[arg(short = 'A', long = "after", help = "Lines of context after task")]
        after: Option<usize>,
        #[arg(short = 'B', long = "before", help = "Lines of context before task")]
        before: Option<usize>,
        #[arg(short = 'C', long = "context", help = "Lines of context on both sides")]
        context: Option<usize>,
        #[arg(short = 'b', long = "breadcrumbs", help = "Show heading hierarchy")]
        breadcrumbs: bool,
        #[arg(long = "group", value_parser = ["file", "date"], help = "Group results")]
        group: Option<String>,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// List tags.
    Tags {
        file: Option<String>,
        #[arg(long = "find")]
        find: Option<String>,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    // Neither of these reads a vault, and both have to work where no config
    // exists yet -- `init` is how a config comes to exist at all, so loading
    // one first would make it impossible to run.
    match &cli.command {
        Command::Init { force } => return notes_cmd::init(*force),
        Command::SelfUpdate { check, yes } => return update::run(*check, *yes),
        _ => {}
    }

    let config = vault::load_config(cli.config.as_deref(), cli.vault.as_deref())?;

    match cli.command {
        Command::Links {
            file,
            after,
            before,
            format,
        } => commands::links(&config, &file, &format, before, after),
        Command::Backlinks {
            file,
            after,
            before,
            format,
        } => commands::backlinks(&config, &file, &format, before, after),
        Command::Orphans {
            include_special,
            format,
        } => commands::orphans(&config, &format, include_special),
        Command::Hubs { limit, format } => commands::hubs(&config, limit, &format),
        Command::BrokenLinks { format } => commands::broken_links(&config, &format),
        Command::Init { .. } | Command::SelfUpdate { .. } => unreachable!("dispatched above"),
        Command::Context {
            file,
            no_content,
            no_backlinks,
            no_tasks,
            max_content,
            format,
        } => notes_cmd::context(
            &config,
            &file,
            &format,
            &notes_cmd::ContextOptions {
                no_content,
                no_backlinks,
                no_tasks,
                max_content,
            },
        ),
        Command::Frontmatter(FrontmatterCommand::Get { file, key, format }) => {
            notes_cmd::frontmatter_get(&config, &file, key.as_deref(), &format)
        }
        Command::Frontmatter(FrontmatterCommand::Set { file, key, value }) => {
            notes_cmd::frontmatter_set(&config, &file, &key, &value)
        }
        Command::Frontmatter(FrontmatterCommand::Delete { file, key }) => {
            notes_cmd::frontmatter_delete(&config, &file, &key)
        }
        Command::Move {
            source,
            destination,
            dry_run,
            format,
        } => notes_cmd::move_note(&config, &source, &destination, dry_run, &format),
        Command::Lint { check, format } => notes_cmd::lint(&config, &check, &format),
        Command::Daily {
            date,
            path_only,
            format,
        } => notes_cmd::daily(&config, date.as_deref(), path_only, &format),
        Command::Rename {
            old,
            new,
            dry_run,
            format,
        } => notes_cmd::rename(&config, &old, &new, dry_run, &format),
        Command::Query {
            r#where,
            field,
            from,
            sort,
            limit,
            format,
        } => {
            let args = query::QueryArgs {
                where_: &r#where,
                fields: &field,
                from: from.as_deref(),
                sort: sort.as_deref(),
                limit,
                format: &format,
            };
            // A malformed filter is a usage error, which is exit 2.
            if let Err(err) = query::query(&config, &args) {
                eprintln!("Error: {err}");
                std::process::exit(2);
            }
            Ok(())
        }
        Command::Fields { format } => query::fields(&config, &format),
        Command::Tasks {
            action: Some(action),
            ..
        } => match action {
            TaskCommand::New {
                text,
                file,
                due,
                recurring,
                priority,
            } => tasks::new_task(
                &config,
                &text,
                file.as_deref(),
                due.as_deref(),
                recurring.as_deref(),
                priority.as_deref(),
            ),
            TaskCommand::Done {
                text,
                file,
                partial,
            } => tasks::set_status(&config, "done", &text, file.as_deref(), partial),
            TaskCommand::Wip {
                text,
                file,
                partial,
            } => tasks::set_status(&config, "wip", &text, file.as_deref(), partial),
            TaskCommand::Cancel {
                text,
                file,
                partial,
            } => tasks::set_status(&config, "cancel", &text, file.as_deref(), partial),
            TaskCommand::Set {
                status,
                text,
                file,
                partial,
            } => tasks::set_status(&config, &status, &text, file.as_deref(), partial),
        },
        Command::Tasks {
            action: None,
            all,
            done,
            recurring,
            overdue,
            has_date,
            due_on,
            due_from,
            due_to,
            created_from,
            created_to,
            done_from,
            done_to,
            file,
            exclude,
            tag,
            status,
            after,
            before,
            context,
            breadcrumbs,
            group,
            format,
        } => {
            let filters = tasks::Filters {
                include_done: all,
                only_done: done,
                recurring_only: recurring,
                overdue,
                has_date,
                file: file.as_deref(),
                exclude: &exclude,
                tag: tag.as_deref(),
                status: &status,
                due_on: due_on.as_deref(),
                due_from: due_from.as_deref(),
                due_to: due_to.as_deref(),
                created_from: created_from.as_deref(),
                created_to: created_to.as_deref(),
                done_from: done_from.as_deref(),
                done_to: done_to.as_deref(),
                context_before: before.or(context).unwrap_or(0),
                context_after: after.or(context).unwrap_or(0),
            };
            // A rejected filter value is a usage error, which is exit 2.
            let found = match tasks::find_tasks(&config, &filters) {
                Ok(found) => found,
                Err(err) => {
                    eprintln!("Error: {err}");
                    std::process::exit(2);
                }
            };
            tasks::print_tasks(&found, &format, &tasks::Display { breadcrumbs, group });
            Ok(())
        }
        Command::Tags { file, find, format } => {
            commands::tags(&config, file.as_deref(), find.as_deref(), &format)
        }
    }
}
