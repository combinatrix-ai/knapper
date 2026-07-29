//! Template expansion for daily notes.
//!
//! Both engines use moment-style date tokens; templater adds `<% %>` blocks.

use std::sync::LazyLock;

use chrono::{Duration, Local, NaiveDate, Timelike};
use regex::Regex;

// The format argument is optional: `tp.date.now()` is as common in real
// templates as the spelled-out form.
static TP_DATE_NOW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<%\s*tp\.date\.now\(\s*(?:"([^"]*)"\s*(?:,\s*(-?\d+)\s*)?)?\)\s*%>"#).unwrap()
});
static TP_DATE_SHIFT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<%\s*tp\.date\.(yesterday|tomorrow)\(\s*(?:"([^"]*)"\s*)?\)\s*%>"#).unwrap()
});
static TP_FILE_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<%\s*tp\.file\.title\s*%>").unwrap());
static CORE_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{date(?::([^}]+))?\}\}").unwrap());
static CORE_TIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{time(?::([^}]+))?\}\}").unwrap());
static CORE_TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{title\}\}").unwrap());

const DEFAULT_DATE_FORMAT: &str = "YYYY-MM-DD";

fn format_date(format: &str, date: NaiveDate) -> String {
    let format = if format.is_empty() {
        DEFAULT_DATE_FORMAT
    } else {
        format
    };
    format
        .replace("YYYY", &date.format("%Y").to_string())
        .replace("MM", &date.format("%m").to_string())
        .replace("DD", &date.format("%d").to_string())
}

/// `{{time}}` means the moment the note is being created, which is now even
/// when the note itself is dated in the past (`knapper daily yesterday`).
fn format_time(format: &str) -> String {
    let now = Local::now();
    let format = if format.is_empty() { "HH:mm" } else { format };
    format
        .replace("HH", &format!("{:02}", now.hour()))
        .replace("mm", &format!("{:02}", now.minute()))
        .replace("ss", &format!("{:02}", now.second()))
}

pub fn expand(content: &str, engine: &str, title: &str, date: NaiveDate) -> String {
    let mut out = content.to_string();

    if engine != "core" {
        out = TP_DATE_NOW
            .replace_all(&out, |c: &regex::Captures| {
                let offset: i64 = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
                let format = c.get(1).map(|m| m.as_str()).unwrap_or(DEFAULT_DATE_FORMAT);
                format_date(format, date + Duration::days(offset))
            })
            .into_owned();
        out = TP_DATE_SHIFT
            .replace_all(&out, |c: &regex::Captures| {
                let days = if &c[1] == "yesterday" { -1 } else { 1 };
                let format = c.get(2).map(|m| m.as_str()).unwrap_or(DEFAULT_DATE_FORMAT);
                format_date(format, date + Duration::days(days))
            })
            .into_owned();
        out = TP_FILE_TITLE.replace_all(&out, title).into_owned();
    }

    out = CORE_DATE
        .replace_all(&out, |c: &regex::Captures| {
            format_date(c.get(1).map(|m| m.as_str()).unwrap_or(""), date)
        })
        .into_owned();
    out = CORE_TIME
        .replace_all(&out, |c: &regex::Captures| {
            format_time(c.get(1).map(|m| m.as_str()).unwrap_or(""))
        })
        .into_owned();
    CORE_TITLE.replace_all(&out, title).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 26).unwrap()
    }

    fn templater(content: &str) -> String {
        expand(content, "templater", "Test Note", date())
    }

    #[test]
    fn dates_format_with_moment_tokens() {
        assert_eq!(format_date("YYYY-MM-DD", date()), "2026-01-26");
        assert_eq!(format_date("YYYY年MM月DD日", date()), "2026年01月26日");
    }

    #[test]
    fn tp_date_now_works_with_and_without_a_format() {
        assert_eq!(templater("<% tp.date.now() %>"), "2026-01-26");
        assert_eq!(
            templater(r#"<% tp.date.now("YYYY/MM/DD") %>"#),
            "2026/01/26"
        );
    }

    #[test]
    fn tp_date_now_takes_a_day_offset() {
        assert_eq!(
            templater(r#"<% tp.date.now("YYYY-MM-DD", -1) %>"#),
            "2026-01-25"
        );
        assert_eq!(
            templater(r#"<% tp.date.now("YYYY-MM-DD", 7) %>"#),
            "2026-02-02"
        );
    }

    #[test]
    fn yesterday_and_tomorrow_are_shorthands() {
        assert_eq!(templater("<% tp.date.yesterday() %>"), "2026-01-25");
        assert_eq!(templater("<% tp.date.tomorrow() %>"), "2026-01-27");
        assert_eq!(
            templater(r#"<% tp.date.tomorrow("YYYY/MM/DD") %>"#),
            "2026/01/27"
        );
    }

    #[test]
    fn the_file_title_expands() {
        assert_eq!(templater("<% tp.file.title %>"), "Test Note");
    }

    /// Leaving an unrecognised expression alone is better than eating it:
    /// the template stays visible and fixable.
    #[test]
    fn an_unknown_expression_is_left_untouched() {
        assert_eq!(
            templater("<% tp.unknown.thing() %>"),
            "<% tp.unknown.thing() %>"
        );
    }

    #[test]
    fn several_expressions_expand_in_one_pass() {
        assert_eq!(
            templater("[[<% tp.date.now() %>]] to [[<% tp.date.tomorrow() %>]]"),
            "[[2026-01-26]] to [[2026-01-27]]"
        );
    }

    #[test]
    fn core_template_tokens_expand_under_either_engine() {
        for engine in ["templater", "core"] {
            assert_eq!(expand("{{date}}", engine, "My Note", date()), "2026-01-26");
            assert_eq!(
                expand("{{date:YYYY年MM月DD日}}", engine, "My Note", date()),
                "2026年01月26日"
            );
            assert_eq!(expand("{{title}}", engine, "My Note", date()), "My Note");
        }
    }

    /// `{{time}}` is the creation moment, so its value cannot be pinned --
    /// but its shape can, and that it is no longer the frozen 00:00 it used
    /// to be for every note.
    #[test]
    fn core_time_expands_to_a_real_clock_time() {
        let out = expand("{{time}}", "core", "T", date());
        assert_eq!(out.len(), 5, "expected HH:mm, got {out:?}");
        assert_eq!(&out[2..3], ":");
        assert!(
            out.chars().filter(char::is_ascii_digit).count() == 4,
            "got {out:?}"
        );

        let with_seconds = expand("{{time:HH:mm:ss}}", "core", "T", date());
        assert_eq!(
            with_seconds.len(),
            8,
            "expected HH:mm:ss, got {with_seconds:?}"
        );
    }

    /// Under the core engine, templater blocks are not the engine's syntax
    /// and must survive untouched.
    #[test]
    fn the_core_engine_leaves_templater_blocks_alone() {
        assert_eq!(
            expand("<% tp.date.now() %>", "core", "T", date()),
            "<% tp.date.now() %>"
        );
    }
}
