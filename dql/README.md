# Embedded Dataview DQL

The runtime uses the **unchanged Dataview 0.5.70 parser, expression functions,
query evaluator and Markdown field importer** at commit
`5ad0994ff384cbb797de382e7edff2388141b73a`. The MIT source and all 18 upstream
test suites are in `../vendor/dataview/`. `engine.ts` adapts a Rust-produced
Markdown/CSV snapshot to the index interfaces required by that engine.

## Use

```sh
knapper dql 'TABLE messages FROM "Diary" SORT file.day' --format json
knapper dql 'TABLE sum(rows.messages) FROM "Diary" GROUP BY status'
knapper dql 'TASK FROM #work WHERE !completed'
knapper dql 'CALENDAR file.day FROM "Diary"'
knapper dql 'TABLE this.messages FROM "Diary"' --origin Diary/2026-10-03.md
knapper dql 'TABLE messages FROM "Diary"' --memory-limit-mib 1024 --timeout-seconds 120
knapper dql 'TABLE count FROM csv("counts.csv")' --origin Diary/2026-10-03.md
knapper dql --licenses
```

`TABLE`, `LIST`, `TASK`, `CALENDAR`, source expressions, `WHERE`, `SORT`,
`LIMIT`, `GROUP BY`, `FLATTEN`, and functions use upstream implementations.
`--origin` supplies the current page for `this` and relative CSV paths.
`--timezone` selects an IANA timezone; the default is the host timezone.
The existing `knapper query` command retains its own metadata/filter behavior.

The JSON output has `type`, `headers`, `values`, `tasks` and `diagnostics`.
Dates, durations and links retain type tags instead of silently turning into
strings. Partial row errors appear in diagnostics and stderr; an all-row error
fails the command. Text tables are emitted as Markdown; TASK text output is
currently structured JSON.

## Compatibility boundary

This is a **headless DQL integration**, not a claim of full Obsidian compatibility.
The canonical feature contracts and verification status are in [spec.md](../spec.md).

- DQL query/evaluation semantics are pinned to the vendored version, including
  its quirks. Upstream updates require regenerated bundles and reference tests.
- The source snapshot respects knapper's hidden-directory and `exclude` rules.
  Only `.md`/`.markdown` pages and scoped CSV files are indexed; Org and MDX are
  not Dataview pages. CSV sources cannot fetch URLs or files outside this scope.
- CommonMark sections/list nesting and knapper's link/tag scanner provide the
  Obsidian-shaped metadata. Obsidian-specific syntax (such as callout/list
  cache edge cases), ambiguous/case-insensitive link lookup and bookmark state
  are not fully matched. `file.starred` currently returns false.
- Formatting uses bundled English and Japanese locale data, with all FormatJS
  timezone data. It does not reproduce arbitrary Obsidian UI locales. Timezone
  history/future coverage follows that pinned data, not the host ICU database.
- There is no DataviewJS, inline-query rendering, DOM, task checkbox mutation,
  Obsidian plugin API, persistence or live refresh.
- Queries execute in QuickJS with no host I/O callbacks/module loader, a 256 MiB
  default heap limit and a 30-second execution deadline. Large vaults can use
  `--memory-limit-mib` (16–4096 MiB); this limits the JS heap, not total process
  memory. `--timeout-seconds` (1–600) overrides the engine deadline, including
  JS index construction. Rust snapshot construction occurs
  before that deadline and reads scoped Markdown/CSV content.

## Build and verify

**Normal Rust builds need no Node or package manager.** The generated JS
bundles and notices are checked in and embedded with `include_str!`.
QuickJS-NG is statically linked; no JS sidecar, Node, Deno or npm is needed at
runtime. An ordinary C compiler is required to build QuickJS from source.

To change or update the TS engine (Node and pnpm required only here):

```sh
pnpm --dir dql install --frozen-lockfile --ignore-scripts
pnpm --dir dql build
pnpm --dir dql test
cargo test
KNAPPER_DQL_NODE=node cargo test node_reference_matches_embedded_runtime -- --ignored
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo build --release
```

`build.mjs` creates:

- `engine.js`: production engine, dependencies, Intl polyfills and locale/tz data.
- `upstream-tests.js`: **all 395 upstream test bodies/expectations**, with a small
  synchronous Jest-compatible matcher runner for embedded execution. Unknown
  matchers fail; registration is deferred like Jest. This is test-only and is
  not embedded in the production binary.
- `upstream-index.ts`: unchanged IndexMap/PathFilters excerpts generated from the
  upstream source; live Obsidian index setup is omitted.
- `native-engine.js` (ignored): Node-native-Intl reference engine, so the
  differential test can catch runtime/polyfill drift on ten full DQL queries.
- `THIRD_PARTY_NOTICES.txt`: upstream and bundled dependency license texts,
  including the statically linked QuickJS/bridge licenses.

The Rust suite runs all 395 upstream cases inside the actual embedded engine.
Jest separately runs the 18 unchanged suites against Node's native runtime.
End-to-end fixtures verify the metadata bridge, scoping, dates/DST, lists/tasks,
links, CSV, grouping and diagnostics. The ignored Node reference test is run
explicitly in CI. The upstream tests mostly cover parsing and expressions;
they do **not** establish full Obsidian metadata-cache equivalence.

## Vendoring and notices

Dataview and the bundled JS packages are MIT-licensed; retain their notices
when copying or distributing the bundle. `knapper dql --licenses` prints the
notices embedded in the binary even outside a vault. Build and test tools are
not embedded in the release executable. Cargo dependencies retain their own
license obligations; added runtime notices are in `RUST_NOTICES.txt`.
