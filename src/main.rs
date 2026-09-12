//! knapper - a headless CLI for markdown knowledge bases.
//!
//! Reads and writes note files directly; nothing needs to be running.
//! Behaviour is pinned by tests/contract/cases.yaml, which this binary and the
//! Python implementation both answer to.

// A binary test harness replaces `main`, so its command tree appears unused
// even though the production binary reaches it. Keep that suppression out of
// normal builds so genuinely dead production code remains visible.
#![cfg_attr(test, allow(dead_code))]

mod commands;
mod demote;
mod graph;
mod links;
mod move_tree;
mod note;
mod notes_cmd;
mod org;
mod parser;
mod providers;
mod query;
mod refs;
mod repair;
mod skill;
mod tasks;
mod templater;
mod topic;
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
        #[arg(long = "scheduled", help = "Scheduled date (YYYY-MM-DD)")]
        scheduled: Option<String>,
        #[arg(long = "start", help = "Start date (YYYY-MM-DD)")]
        start: Option<String>,
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

#[derive(Subcommand)]
enum ProviderCommand {
    /// List the configured providers and the commands they run.
    List {
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Define or replace one provider's command.
    Set {
        name: String,
        /// The command to run, after `--`. It is argv, not a shell line, and
        /// every `{locator}` in it is replaced with the reference's locator.
        #[arg(
            last = true,
            required = true,
            num_args = 1..,
            value_name = "COMMAND",
            help = "Command after `--`, e.g. -- op read 'op://Vault/{locator}/value'"
        )]
        command: Vec<String>,
    },
    /// Forget one provider.
    Remove { name: String },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Validate the discovered config without scanning the vault.
    Check {
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Print the JSON Schema used by YAML-aware editors.
    Schema,
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
    /// List incoming links to a file, or the notes carrying a '#tag'.
    Backlinks {
        #[arg(
            value_name = "FILE|#TAG",
            help = "A note, or a '#tag' to look up as a topic"
        )]
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
        #[arg(long = "include-special", help = "Include hidden folders")]
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
    /// Find links to missing notes, headings, or block IDs.
    #[command(name = "broken-links")]
    BrokenLinks {
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Plan repairs for broken links. Reports only; writes nothing.
    #[command(
        name = "repair-links",
        long_about = "Plan repairs for links that are already broken.\n\n\
        This is the other half of `rename` and `move`: they keep links intact through a\n\
        refactor knapper performs, and this reports the ones that broke while knapper was\n\
        not looking -- a folder reorganised elsewhere, an exporter's stale path.\n\n\
        A repair is proposed only where the filesystem settles it: exactly one file, reached\n\
        by an exact structural transformation of the target as written. A resemblance is\n\
        not evidence, so a renamed concept, a missing date and a citation label are reported\n\
        and left alone.\n\n\
        V1 is read-only. --dry-run is required, and nothing is ever written."
    )]
    RepairLinks {
        #[arg(long = "dry-run", help = "Required: this command only ever plans")]
        dry_run: bool,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Initialize a new knapper config file.
    Init {
        #[arg(short = 'f', long = "force")]
        force: bool,
    },
    /// Inspect or print the knapper YAML configuration schema.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Print the embedded agent skill, or register it with the agents here.
    Skill {
        /// Write it into every agent host found on this machine.
        #[arg(long = "install")]
        install: bool,
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
    /// Get comprehensive context for a file or a '#tag' (for LLMs).
    Context {
        #[arg(
            value_name = "FILE|#TAG",
            help = "A note, or a '#tag' to read as a virtual topic"
        )]
        file: String,
        #[arg(long = "no-content", help = "Exclude file content")]
        no_content: bool,
        #[arg(long = "no-backlinks", help = "Exclude backlinks (faster)")]
        no_backlinks: bool,
        #[arg(long = "no-tasks", help = "Exclude tasks")]
        no_tasks: bool,
        #[arg(long = "max-content", help = "Truncate content to N characters")]
        max_content: Option<usize>,
        #[arg(
            long = "line",
            value_name = "N",
            help = "Focus on physical 1-based line N"
        )]
        line: Option<usize>,
        #[arg(
            short = 'B',
            long = "before",
            value_name = "N",
            help = "Lines before the focused line (default: 3)"
        )]
        before: Option<usize>,
        #[arg(
            short = 'A',
            long = "after",
            value_name = "N",
            help = "Lines after the focused line (default: 3)"
        )]
        after: Option<usize>,
        #[arg(
            long = "section",
            help = "Show the smallest enclosing Markdown section"
        )]
        section: bool,
        #[arg(
            long = "outline-depth",
            value_name = "N",
            help = "Focused document map depth (default: 1; 0 hides it)"
        )]
        outline_depth: Option<usize>,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Read and write frontmatter.
    #[command(subcommand)]
    Frontmatter(FrontmatterCommand),
    /// Check vault health.
    Lint {
        #[arg(
            long = "check",
            value_name = "RULE",
            value_parser = ["broken-links", "orphans", "duplicates", "empty", "frontmatter", "headings"],
            help = "Run one configured lint rule (repeatable)"
        )]
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
    /// Rewrite a hard [[target]] into a soft #tag.
    #[command(long_about = "Demote a wikilink target to a tag.\n\n\
        [[X]] is a hard note reference: a missing target is a broken link. #X is a soft\n\
        topic reference: it labels a note and promises no note exists. This rewrites the\n\
        exact form -- [[X]] becomes #X -- and reports every occurrence it will not touch,\n\
        such as [[X|alias]], [[X#heading]], ![[X]] and [[folder/X]].")]
    Demote {
        #[arg(value_name = "TARGET", help = "A wikilink target: X or [[X]]")]
        target: String,
        #[arg(long = "tag", help = "Write this tag instead of the target itself")]
        tag: Option<String>,
        #[arg(long = "dry-run")]
        dry_run: bool,
        #[arg(
            long = "allow-existing-note",
            help = "Demote even though the target names a note that exists"
        )]
        allow_existing_note: bool,
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
    /// Move a note or a directory to a different folder and update all links.
    Move {
        source: String,
        destination: String,
        #[arg(long = "dry-run")]
        dry_run: bool,
        #[arg(
            long = "allow-broken-org-links",
            help = "Move a directory even though inbound org links will break"
        )]
        allow_broken_org_links: bool,
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
        #[arg(
            long = "available",
            help = "Hide tasks whose start date is after today"
        )]
        available: bool,
        #[arg(long = "due-on")]
        due_on: Option<String>,
        #[arg(long = "due-from")]
        due_from: Option<String>,
        #[arg(long = "due-to")]
        due_to: Option<String>,
        #[arg(long = "scheduled-on")]
        scheduled_on: Option<String>,
        #[arg(long = "scheduled-from")]
        scheduled_from: Option<String>,
        #[arg(long = "scheduled-to")]
        scheduled_to: Option<String>,
        #[arg(long = "start-on")]
        start_on: Option<String>,
        #[arg(long = "start-from")]
        start_from: Option<String>,
        #[arg(long = "start-to")]
        start_to: Option<String>,
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
        #[arg(
            long = "prose-only",
            help = "Skip checkboxes in code fences, %%comments%% and inline code"
        )]
        prose_only: bool,
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
    /// List external references (knapper://provider/locator) in the vault.
    Refs {
        file: Option<String>,
        #[arg(long = "provider", help = "Only references naming this provider")]
        provider: Option<String>,
        #[arg(short = 'f', long = "format", default_value = "text")]
        format: String,
    },
    /// Read the value behind one reference, through its provider's command.
    Resolve {
        #[arg(value_name = "REFERENCE", help = "knapper://<provider>/<locator>")]
        reference: String,
        #[arg(long = "timeout", value_name = "SECS", help = "Give up after SECS")]
        timeout: Option<f64>,
        #[arg(long = "dry-run", help = "Print the command instead of running it")]
        dry_run: bool,
    },
    /// Configure the commands that resolve references.
    #[command(subcommand)]
    Provider(ProviderCommand),
}

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

/// A broker failure carries the exit status that tells a malformed reference
/// (2) from an unknown provider (3) from a command that would not run (4).
/// Reporting it here keeps `exit()` out of the modules, next to the usage
/// errors the other commands already report this way.
fn broker(result: std::result::Result<(), providers::Failure>) -> Result<()> {
    if let Err(err) = result {
        eprintln!("Error: {err}");
        std::process::exit(err.exit_code());
    }
    Ok(())
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    // None of these reads a vault, and the first has to work where no config
    // exists yet -- `init` is how a config comes to exist at all, so loading
    // one first would make it impossible to run. The broker commands are
    // about the local provider config rather than about any note, so they
    // work from anywhere too.
    match &cli.command {
        Command::Init { force } => return notes_cmd::init(*force),
        Command::Config(ConfigCommand::Schema) => return notes_cmd::config_schema(),
        Command::Config(ConfigCommand::Check { format }) => {
            return notes_cmd::config_check(cli.config.as_deref(), cli.vault.as_deref(), format)
        }
        Command::Skill { install } => return skill::run(*install),
        Command::SelfUpdate { check, yes } => return update::run(*check, *yes),
        Command::Provider(ProviderCommand::List { format }) => {
            return broker(providers::list(format))
        }
        Command::Provider(ProviderCommand::Set { name, command }) => {
            return broker(providers::set(name, command))
        }
        Command::Provider(ProviderCommand::Remove { name }) => {
            return broker(providers::remove(name))
        }
        Command::Resolve {
            reference,
            timeout,
            dry_run,
        } => {
            return broker(providers::resolve(
                reference,
                &providers::ResolveOptions {
                    timeout: *timeout,
                    dry_run: *dry_run,
                },
            ))
        }
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
        // A leading `#` is the only thing that turns a subject argument into
        // a tag, so a tag is never resolved by accident and a note argument
        // reaches exactly the code it always did.
        Command::Backlinks {
            file,
            after,
            before,
            format,
        } => match topic::Selector::parse(&file)? {
            topic::Selector::Tag(tag) => topic::backlinks(&config, &tag, &format, before, after),
            topic::Selector::Note(file) => {
                commands::backlinks(&config, &file, &format, before, after)
            }
        },
        Command::Orphans {
            include_special,
            format,
        } => commands::orphans(&config, &format, include_special),
        Command::Hubs { limit, format } => commands::hubs(&config, limit, &format),
        Command::BrokenLinks { format } => commands::broken_links(&config, &format),
        // Omitting --dry-run is a usage error, which is exit 2 -- and it is
        // refused before anything reads the vault, so there is no path on
        // which a missing flag could be read as consent to write.
        Command::RepairLinks { dry_run, format } => {
            if !dry_run {
                eprintln!(
                    "Error: repair-links only plans repairs and never writes. \
                     Re-run with --dry-run."
                );
                std::process::exit(2);
            }
            repair::repair_links(&config, dry_run, &format)
        }
        Command::Init { .. }
        | Command::Config(..)
        | Command::Skill { .. }
        | Command::SelfUpdate { .. }
        | Command::Provider(..)
        | Command::Resolve { .. } => {
            unreachable!("dispatched above")
        }
        Command::Context {
            file,
            no_content,
            no_backlinks,
            no_tasks,
            max_content,
            line,
            before,
            after,
            section,
            outline_depth,
            format,
        } => {
            let options = notes_cmd::ContextOptions {
                no_content,
                no_backlinks,
                no_tasks,
                max_content,
                line,
                before,
                after,
                section,
                outline_depth,
            };
            match topic::Selector::parse(&file)? {
                topic::Selector::Tag(tag) => topic::context(&config, &tag, &format, &options),
                topic::Selector::Note(file) => {
                    notes_cmd::context(&config, &file, &format, &options)
                }
            }
        }
        Command::Frontmatter(FrontmatterCommand::Get { file, key, format }) => {
            notes_cmd::frontmatter_get(&config, &file, key.as_deref(), &format)
        }
        Command::Frontmatter(FrontmatterCommand::Set { file, key, value }) => {
            notes_cmd::frontmatter_set(&config, &file, &key, &value)
        }
        Command::Frontmatter(FrontmatterCommand::Delete { file, key }) => {
            notes_cmd::frontmatter_delete(&config, &file, &key)
        }
        // One verb, two engines. A directory move is not a note move with a
        // wider net: what moves is a subtree, and what has to be rewritten is
        // every link that resolves into it, which is a different question
        // from "what mentions this name".
        Command::Move {
            source,
            destination,
            dry_run,
            allow_broken_org_links,
            format,
        } => {
            if move_tree::looks_like_directory(&config, &source) {
                move_tree::move_directory(
                    &config,
                    &source,
                    &destination,
                    &move_tree::Options {
                        dry_run,
                        allow_broken_org_links,
                    },
                    &format,
                )
            } else {
                notes_cmd::move_note(&config, &source, &destination, dry_run, &format)
            }
        }
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
        Command::Demote {
            target,
            tag,
            dry_run,
            allow_existing_note,
            format,
        } => demote::demote(
            &config,
            &target,
            &demote::Options {
                dry_run,
                tag,
                allow_existing_note,
            },
            &format,
        ),
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
                scheduled,
                start,
                recurring,
                priority,
            } => tasks::new_task(
                &config,
                &text,
                file.as_deref(),
                due.as_deref(),
                scheduled.as_deref(),
                start.as_deref(),
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
            available,
            due_on,
            due_from,
            due_to,
            scheduled_on,
            scheduled_from,
            scheduled_to,
            start_on,
            start_from,
            start_to,
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
            prose_only,
            format,
        } => {
            let filters = tasks::Filters {
                include_done: all,
                only_done: done,
                recurring_only: recurring,
                overdue,
                has_date,
                available,
                file: file.as_deref(),
                exclude: &exclude,
                tag: tag.as_deref(),
                status: &status,
                due_on: due_on.as_deref(),
                due_from: due_from.as_deref(),
                due_to: due_to.as_deref(),
                scheduled_on: scheduled_on.as_deref(),
                scheduled_from: scheduled_from.as_deref(),
                scheduled_to: scheduled_to.as_deref(),
                start_on: start_on.as_deref(),
                start_from: start_from.as_deref(),
                start_to: start_to.as_deref(),
                created_from: created_from.as_deref(),
                created_to: created_to.as_deref(),
                done_from: done_from.as_deref(),
                done_to: done_to.as_deref(),
                context_before: before.or(context).unwrap_or(0),
                context_after: after.or(context).unwrap_or(0),
                prose_only,
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
        Command::Refs {
            file,
            provider,
            format,
        } => {
            // A rejected provider name is a usage error, which is exit 2. A
            // missing file is not, and reports like every other file command.
            if let Some(name) = &provider {
                broker(
                    refs::validate_provider(name)
                        .map_err(|err| providers::Failure::Usage(err.to_string())),
                )?;
            }
            refs::refs(&config, file.as_deref(), provider.as_deref(), &format)
        }
    }
}
