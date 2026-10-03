//! End-to-end checks for the headless metadata/index bridge, beyond upstream
//! parser/expression tests. All fixtures are synthetic; no account access.
use serde_json::{json, Value};
use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

fn fixture() -> TempDir {
    let vault = TempDir::new().unwrap();
    fs::create_dir(vault.path().join("Diary")).unwrap();
    fs::create_dir(vault.path().join("Excluded")).unwrap();
    fs::write(
        vault.path().join("knapper.yaml"),
        "vault_path: .\nexclude: [Excluded]\n",
    )
    .unwrap();
    fs::write(vault.path().join("Diary/2026-10-02.md"), "---\ntags: [mail/history]\nstatus: open\nrelated: '[[2026-10-03]]'\n---\n# Mail\nmessages:: 800\n[accounts:: [[Personal]], [[Work]]]\n- [ ] respond [cost:: 2]\n  - [x] inspect\n- [/] pending\n\n[[2026-10-03|Tomorrow]]\n![[2026-10-03#Mail]]\n").unwrap();
    fs::write(vault.path().join("Diary/2026-10-03.md"), "---\ntags: [mail/history]\nstatus: open\n---\n# Mail\nmessages:: 831\n[accounts:: [[Work]]]\n- [x] all done\n").unwrap();
    fs::write(
        vault.path().join("Other.md"),
        "---\nstatus: closed\n---\nmessages:: 9\n",
    )
    .unwrap();
    fs::write(
        vault.path().join("Excluded/2026-10-04.md"),
        "messages:: 9999\n",
    )
    .unwrap();
    fs::write(
        vault.path().join("Diary/counts.csv"),
        "date,count\n2026-10-02,800\n2026-10-03,831\n",
    )
    .unwrap();
    fs::write(vault.path().join("Excluded/secret.csv"), "count\n9999\n").unwrap();
    vault
}

fn run(vault: &TempDir, query: &str, extras: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_knapper"))
        .current_dir(vault.path())
        .env("KNAPPER_NO_UPDATE_CHECK", "1")
        .args(["dql", query, "--format", "json", "--timezone", "Asia/Tokyo"])
        .args(extras)
        .output()
        .unwrap()
}
fn query(vault: &TempDir, query: &str, extras: &[&str]) -> Value {
    let output = run(vault, query, extras);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn daily_counts_sort_filter_limit_and_dates() {
    let v = fixture();
    let result = query(&v, "TABLE WITHOUT ID file.name, messages, file.day FROM \"Diary\" WHERE messages > 800 SORT file.day DESC LIMIT 1", &[]);
    assert_eq!(
        result["headers"],
        json!(["file.name", "messages", "file.day"])
    );
    assert_eq!(
        result["values"],
        json!([["2026-10-03",831,{"type":"date","value":"2026-10-03T00:00:00.000+09:00"}]])
    );
}
#[test]
fn group_aggregate_uses_upstream_pipeline() {
    let v = fixture();
    let r = query(&v,"TABLE length(rows) AS Days, sum(rows.messages) AS Total, average(rows.messages) AS Mean FROM \"Diary\" GROUP BY status",&[]);
    assert_eq!(r["values"], json!([["open", 2, 1631, 815.5]]));
}

#[test]
fn japanese_paragraph_and_list_endings_preserve_metadata() {
    let v = fixture();
    fs::write(
        v.path().join("Diary/2026-10-04.md"),
        "# 件数履歴\n日本語の行末。\n\nmessages:: 9\n\n- 日本語の項目。\n",
    )
    .unwrap();
    let r = query(
        &v,
        "TABLE WITHOUT ID messages, file.lists.text FROM \"Diary/2026-10-04.md\"",
        &["--memory-limit-mib", "256"],
    );
    assert_eq!(r["values"], json!([[9, ["日本語の項目。"]]]));
}

#[test]
fn native_bookmarks_follow_dataview_file_and_group_semantics() {
    let v = fixture();
    fs::create_dir(v.path().join(".obsidian")).unwrap();
    let bookmarks = json!({"items":[
        {"type":"file","path":"Diary/2026-10-02.md","subpath":"#Mail"},
        {"type":"group","items":[{"type":"group","items":[{"type":"file","path":"Diary/2026-10-03.md","subpath":"#^block"}]}]},
        {"type":"file","path":"Diary/2026-10-02.md"},
        {"type":"file","path":"Excluded/2026-10-04.md"},
        {"type":"folder","path":"Diary"},
        {"type":"search","query":"Other"},
        {"type":"graph","path":"Other.md"},
        {"type":"url","url":"https://example.com"}
    ]});
    fs::write(
        v.path().join(".obsidian/bookmarks.json"),
        bookmarks.to_string(),
    )
    .unwrap();
    let r = query(
        &v,
        "TABLE WITHOUT ID file.path, file.starred SORT file.path",
        &[],
    );
    assert_eq!(
        r["values"],
        json!([
            ["Diary/2026-10-02.md", true],
            ["Diary/2026-10-03.md", true],
            ["Other.md", false]
        ])
    );
    assert_eq!(
        query(
            &v,
            "LIST WITHOUT ID file.path WHERE file.starred SORT file.path",
            &[]
        )["values"],
        json!(["Diary/2026-10-02.md", "Diary/2026-10-03.md"])
    );
}

#[test]
fn bookmark_plugin_state_is_respected_in_both_saved_formats() {
    let v = fixture();
    fs::create_dir(v.path().join(".obsidian")).unwrap();
    let count = "TABLE WITHOUT ID file.path WHERE file.starred";
    for state in [json!({"bookmarks":false}), json!([])] {
        fs::write(
            v.path().join(".obsidian/core-plugins.json"),
            state.to_string(),
        )
        .unwrap();
        fs::write(
            v.path().join(".obsidian/bookmarks.json"),
            "malformed but disabled",
        )
        .unwrap();
        assert_eq!(query(&v, count, &[])["values"], json!([]));
    }
    for state in [json!({"bookmarks":true}), json!(["bookmarks"]), json!({})] {
        fs::write(
            v.path().join(".obsidian/core-plugins.json"),
            state.to_string(),
        )
        .unwrap();
        fs::write(
            v.path().join(".obsidian/bookmarks.json"),
            json!({"items":[{"type":"file","path":"Other.md"}]}).to_string(),
        )
        .unwrap();
        assert_eq!(query(&v, count, &[])["values"], json!([["Other.md"]]));
    }
}

#[test]
fn bookmark_missing_malformed_and_custom_settings_are_explicit() {
    let v = fixture();
    let count = "LIST WITHOUT ID file.path WHERE file.starred";
    assert_eq!(query(&v, count, &[])["values"], json!([]));
    fs::create_dir(v.path().join(".settings")).unwrap();
    let path = v.path().join(".settings/bookmarks.json");
    fs::write(
        &path,
        json!({"items":[{"type":"file","path":"Other.md"}]}).to_string(),
    )
    .unwrap();
    assert_eq!(query(&v, count, &[])["values"], json!([]));
    assert_eq!(
        query(&v, count, &["--obsidian-config-dir", ".settings"])["values"],
        json!(["Other.md"])
    );
    for invalid in [
        "{",
        r#"{"items":null}"#,
        r#"{"items":[{"type":"file"}]}"#,
        r#"{"items":[{"type":"group","items":{}}]}"#,
    ] {
        fs::write(&path, invalid).unwrap();
        let output = run(&v, count, &["--obsidian-config-dir", ".settings"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("bookmarks.json"));
    }
    let output = run(&v, count, &["--obsidian-config-dir", "../outside"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("vault-relative"));
}

#[cfg(unix)]
#[test]
fn bookmark_settings_symlinks_cannot_escape_the_vault() {
    let v = fixture();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("bookmarks.json"), "{}").unwrap();
    std::os::unix::fs::symlink(outside.path(), v.path().join(".outside")).unwrap();
    let output = run(
        &v,
        "LIST WHERE file.starred",
        &["--obsidian-config-dir", ".outside"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("inside the vault"));
}

#[test]
fn committed_actual_obsidian_bookmark_results_are_reproduced() {
    fn copy_directory(source: &std::path::Path, destination: &std::path::Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_directory(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/obsidian-bookmarks");
    let reference: Value =
        serde_json::from_str(&fs::read_to_string(source.join("reference-results.json")).unwrap())
            .unwrap();
    let vault = TempDir::new().unwrap();
    copy_directory(&source, vault.path());
    for state in reference["states"].as_array().unwrap() {
        // A fresh headless snapshot intentionally excludes retained live-plugin
        // data. The actual warm-disable difference is preserved in spec.md.
        if state["lifecycle"] == "warm-disable-after-refresh" {
            continue;
        }
        fs::write(
            vault.path().join(".obsidian/core-plugins.json"),
            json!({"bookmarks":state["enabled"]}).to_string(),
        )
        .unwrap();
        for expected in state["rows"].as_array().unwrap() {
            let actual = query(&vault, expected["query"].as_str().unwrap(), &[]);
            assert_eq!(actual["type"], expected["type"]);
            assert_eq!(actual["headers"], expected["headers"]);
            assert_eq!(actual["values"], expected["values"]);
        }
    }
}
#[test]
fn flatten_typed_inline_links_and_nested_tags() {
    let v = fixture();
    let r = query(&v,"TABLE WITHOUT ID meta(account).path AS Account, sum(rows.messages) AS Total FROM #MAIL FLATTEN accounts AS account GROUP BY account SORT account",&[]);
    assert_eq!(r["values"], json!([["Personal", 800], ["Work", 1631]]));
}
#[test]
fn links_backlinks_and_origin() {
    let v = fixture();
    let r = query(
        &v,
        "LIST WITHOUT ID file.name FROM [[Diary/2026-10-03]]",
        &[],
    );
    assert_eq!(r["values"], json!(["2026-10-02"]));
    let r = query(&v,"TABLE WITHOUT ID related.messages, this.messages, length(file.inlinks) FROM \"Diary/2026-10-02\"",&["--origin","Diary/2026-10-03.md"]);
    assert_eq!(r["values"], json!([[831, 831, 0]]));
    let r = query(
        &v,
        "LIST WITHOUT ID file.name FROM outgoing([[Diary/2026-10-02]])",
        &[],
    );
    assert_eq!(r["values"], json!(["2026-10-03"]));
}
#[test]
fn task_metadata_preserves_hierarchy_custom_status_and_own_text() {
    let v = fixture();
    let r = query(&v, "TASK FROM #mail WHERE !completed SORT line", &[]);
    assert_eq!(r["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(r["tasks"][0]["text"], "respond [cost:: 2]");
    assert_eq!(r["tasks"][0]["cost"], 2);
    assert_eq!(r["tasks"][0]["fullyCompleted"], false);
    assert_eq!(r["tasks"][0]["children"][0]["completed"], true);
    assert_eq!(r["tasks"][1]["status"], "/");
    assert_eq!(r["tasks"][1]["checked"], true);
    assert_eq!(r["tasks"][1]["completed"], false);
}
#[test]
fn calendar_returns_typed_dates_and_links() {
    let v = fixture();
    let r = query(&v, "CALENDAR file.day FROM \"Diary\" SORT file.day", &[]);
    assert_eq!(r["type"], "calendar");
    assert_eq!(
        r["values"][0]["date"]["value"],
        "2026-10-02T00:00:00.000+09:00"
    );
    assert_eq!(r["values"][1]["link"]["path"], "Diary/2026-10-03.md");
}
#[test]
fn relative_csv_is_typed_and_exclusions_are_respected() {
    let v = fixture();
    let r = query(
        &v,
        "TABLE WITHOUT ID sum(rows.count) AS Total FROM csv(\"counts.csv\") GROUP BY true",
        &["--origin", "Diary/2026-10-02.md"],
    );
    assert_eq!(r["values"], json!([[1631]]));
    let result = run(&v, "TABLE count FROM csv(\"Excluded/secret.csv\")", &[]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("CSV not found"));
    let r = query(
        &v,
        "LIST WITHOUT ID file.name FROM #mail AND !\"Excluded\" SORT file.name",
        &[],
    );
    assert_eq!(r["values"], json!(["2026-10-02", "2026-10-03"]));
}
#[test]
fn duplicate_fields_dates_duration_and_null_match_dataview() {
    let v = fixture();
    fs::write(
        v.path().join("Typed.md"),
        "---\nday: 2026-01-02\nspan: 2 hours\n---\ncount:: 1\ncount:: 2\n",
    )
    .unwrap();
    let r = query(
        &v,
        "TABLE WITHOUT ID count, span.hours, file.day.year, missing FROM \"Typed\"",
        &[],
    );
    assert_eq!(r["values"], json!([[[1, 2], 2, 2026, null]]));
}
#[test]
fn inline_fields_in_examples_and_comments_do_not_leak() {
    let v = fixture();
    fs::write(
        v.path().join("Masked.md"),
        "## 数値\ncount:: 3\n\n```md\ncount:: 999\n```\n\n%%\ncount:: 999\n%%\n\n`[count:: 999]`\n",
    )
    .unwrap();
    let r = query(&v, "TABLE WITHOUT ID count FROM \"Masked\"", &[]);
    assert_eq!(r["values"], json!([[3]]));
}
#[test]
fn timezone_dst_and_date_format_functions() {
    let v = fixture();
    let r = query(&v,"TABLE WITHOUT ID dateformat(file.day, \"yyyy-MM-dd\"), dateformat(date(\"2026-07-01T12:00:00[America/Toronto]\"), \"ZZ\") FROM \"Diary/2026-10-02\"",&[]);
    assert_eq!(r["values"], json!([["2026-10-02", "-04:00"]]));
}
#[test]
fn invalid_queries_origins_and_timezones_fail_clearly() {
    let v = fixture();
    for (q, args) in [
        ("SELECT *", vec![]),
        ("LIST", vec!["--origin", "Missing.md"]),
    ] {
        let result = run(&v, q, &args);
        assert!(!result.status.success());
        assert!(!result.stderr.is_empty());
    }
    let result = Command::new(env!("CARGO_BIN_EXE_knapper"))
        .current_dir(v.path())
        .env("KNAPPER_NO_UPDATE_CHECK", "1")
        .args(["dql", "LIST", "--timezone", "Invalid/Zone"])
        .output()
        .unwrap();
    assert!(!result.status.success());
}
#[test]
fn arbitrary_js_and_host_io_are_not_exposed() {
    let v = fixture();
    let result = run(&v, "TABLE eval(\"process.exit(0)\") FROM \"Diary\"", &[]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("eval"));
}
#[test]
fn partial_row_errors_are_reported_instead_of_silent_loss() {
    let v = fixture();
    fs::write(v.path().join("Bad.md"), "messages:: \"oops\"\n").unwrap();
    let result = run(&v, "TABLE WITHOUT ID round(messages)", &[]);
    assert!(result.status.success());
    let r: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| !d["errors"].as_array().unwrap().is_empty()));
    assert!(String::from_utf8_lossy(&result.stderr).contains("row evaluation error"));
}
#[test]
fn licenses_are_available_without_a_vault() {
    let dir = TempDir::new().unwrap();
    let r = Command::new(env!("CARGO_BIN_EXE_knapper"))
        .current_dir(dir.path())
        .args(["dql", "--licenses"])
        .output()
        .unwrap();
    assert!(r.status.success());
    let text = String::from_utf8(r.stdout).unwrap();
    for name in [
        "Michael Brenan",
        "QuickJS-NG",
        "rquickjs",
        "luxon",
        "@formatjs/intl-datetimeformat",
    ] {
        assert!(text.contains(name));
    }
}

#[test]
fn copied_binary_executes_dql_without_node_or_sidecars() {
    let vault = fixture();
    let directory = TempDir::new().unwrap();
    let binary = directory.path().join("knapper");
    fs::copy(env!("CARGO_BIN_EXE_knapper"), &binary).unwrap();
    let output = Command::new(&binary)
        .current_dir(vault.path())
        .env_clear()
        .env("PATH", "/no-programs")
        .env("KNAPPER_NO_UPDATE_CHECK", "1")
        .args([
            "dql",
            "TABLE WITHOUT ID sum(rows.messages) FROM \"Diary\" GROUP BY true",
            "--format",
            "json",
            "--timezone",
            "Asia/Tokyo",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["values"], json!([[1631]]));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

fn pkm_fixture() -> TempDir {
    fn copy(source: &std::path::Path, destination: &std::path::Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let vault = TempDir::new().unwrap();
    copy(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pkm"),
        vault.path(),
    );
    vault
}

#[test]
fn extensible_pkm_vault_cases_match_committed_expectations() {
    let vault = pkm_fixture();
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(vault.path().join("cases.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["schemaVersion"], 1);
    for case in manifest["cases"].as_array().unwrap() {
        let actual = query(&vault, case["query"].as_str().unwrap(), &[]);
        let expected = &case["expected"];
        if let Some(fields) = case["taskFields"].as_array() {
            let tasks: Vec<Value> = actual["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|task| {
                    let mut selected = serde_json::Map::new();
                    for field in fields {
                        let key = field.as_str().unwrap();
                        selected.insert(key.to_owned(), task[key].clone());
                    }
                    Value::Object(selected)
                })
                .collect();
            assert_eq!(
                json!({"type":actual["type"],"tasks":tasks}),
                *expected,
                "{}",
                case["id"]
            );
        } else {
            for (key, value) in expected.as_object().unwrap() {
                assert_eq!(&actual[key], value, "{}: {key}", case["id"]);
            }
        }
    }
}

#[test]
fn pkm_edits_are_visible_to_fresh_queries() {
    let vault = pkm_fixture();
    let todo = vault.path().join("Tasks/Todo.md");
    let before = query(
        &vault,
        "TASK FROM \"Tasks\" WHERE !completed SORT line",
        &[],
    );
    assert_eq!(before["tasks"].as_array().unwrap().len(), 3);
    let contents = fs::read_to_string(&todo)
        .unwrap()
        .replace("- [ ] Draft outline", "- [x] Draft outline");
    fs::write(&todo, contents).unwrap();
    let after = query(
        &vault,
        "TASK FROM \"Tasks\" WHERE !completed SORT line",
        &[],
    );
    assert_eq!(after["tasks"].as_array().unwrap().len(), 2);
    fs::write(vault.path().join("Daily/2026-10-03.md"), "messages:: 9\n").unwrap();
    assert_eq!(
        query(
            &vault,
            "TABLE WITHOUT ID sum(rows.messages) FROM \"Daily\" GROUP BY true",
            &[]
        )["values"],
        json!([[33]])
    );
    fs::remove_file(vault.path().join("Daily/2026-10-03.md")).unwrap();
    assert_eq!(
        query(
            &vault,
            "TABLE WITHOUT ID sum(rows.messages) FROM \"Daily\" GROUP BY true",
            &[]
        )["values"],
        json!([[24]])
    );
}
