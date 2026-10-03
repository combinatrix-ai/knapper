# Obsidian compatibility specification

Last verified: 2026-10-03. This is the canonical specification for knapper's
Obsidian and plugin compatibility. It describes the current development branch;
feature availability in an installed release must be checked separately.
CLI syntax lives in [docs/CLI.md](docs/CLI.md), and DQL build/licensing instructions
live in [dql/README.md](dql/README.md).

## Contract and status

Compatibility means reproducing a named feature's data and behavior through a
specified knapper command. It does not mean running the Obsidian application,
loading arbitrary plugins, or implementing every feature of an installed plugin.

| Status | Meaning |
|---|---|
| Implemented | The stated subset exists and has fixture tests. This is not a blanket plugin-compatibility claim. |
| Partial | The feature exists, with explicit differences listed below. |
| Unsupported | The current implementation does not provide the behavior. |
| Planned | A future contract or candidate; no current behavior is promised. |

Each compatibility entry must identify its input, command/output, source version
when relevant, known differences, and verification. New plugin integrations must
update this file in the same change as implementation and tests. A plugin name or
an upstream test count alone is not evidence of complete compatibility.

## Shared rules

- Note files remain the source of truth. Queries read a fresh snapshot and do not
  require a running Obsidian instance, persistent index, Node, or JS sidecar.
- `knapper.yaml` determines note scope, exclusions, daily-note configuration,
  templates, and knapper task statuses. DQL imports the explicitly scoped
  Core Bookmarks data described below.
  Other Obsidian/community-plugin settings are not imported automatically.
- Commands may have distinct adapters. Native `query`/`tasks`/link commands and
  `dql` must not be described as having identical parsing or resolution behavior
  unless that specific behavior has been compared.
- DQL is read-only. It does not change a task checkbox, bookmark, plugin setting,
  or note. Existing explicit mutation commands retain their own contracts.
- Only known, documented adapters may interpret plugin data. Installing a plugin
  does not authorize its JavaScript to run inside knapper.

## Feature registry

| ID | Source / feature | Interface | Status | Verification |
|---|---|---|---|---|
| CORE-LINKS | Wikilinks, aliases in links, embeds, local Markdown links | Native graph, context, refactoring commands | Implemented subset | `tests/flavors.rs`, `tests/contract.rs`, link tests |
| CORE-PROPERTIES | YAML properties and note aliases | Native frontmatter/query/graph commands | Implemented subset | Native parser/link tests and Obsidian fixture |
| CORE-TAGS | Body/YAML tags, nested and Unicode tags | Native tag/query commands | Implemented subset | Obsidian fixture and parser tests |
| CORE-DAILY | Daily notes and Core Templates | `daily` | Implemented subset | `tests/daily.rs` |
| CORE-BOOKMARKS | Core Bookmarks plugin | DQL `file.starred` | Implemented file-membership subset; cold-snapshot contract | Four adapter tests and actual-Obsidian enabled/disabled-start comparison |
| DV-FIELDS | Dataview inline fields | Native context/query; upstream importer in DQL | Implemented subsets with different adapters | Parser tests, `tests/dql.rs` |
| DV-DQL | Dataview query language and expression functions | `dql` | Implemented pinned engine | 395 upstream tests in Node and QuickJS |
| DV-INDEX | Obsidian metadata supplied to Dataview | `dql` | Partial | `tests/dql.rs`; actual Obsidian comparison described below |
| DV-RENDER | Query-block and inline-query rendering | None | Unsupported | No rendering adapter |
| DV-JS | DataviewJS and Obsidian plugin API | None | Unsupported | Host I/O and arbitrary-JS checks in `tests/dql.rs` |
| TASKS-DATA | Tasks checkbox/date/priority/recurrence conventions | Native `tasks` commands | Implemented subset | Obsidian fixture and task tests |
| TASKS-QUERY | Tasks query-block language, plugin settings and recurrence execution | None | Unsupported | Reading recurrence text is not executing recurrence |
| TEMPLATER-DATE | Templater date/title helpers | `daily` template expansion | Partial | Template and daily-note tests |

## Core Markdown and metadata

Native link commands recognize `[[Note]]`, `[[folder/Note|label]]`, heading/block
subpaths, embeds, and local Markdown links, including URL-encoded paths. The
link graph is at file granularity; heading/block validation and mutations are
separate operations. External URLs/images do not become note-graph edges.
Aliases declared in YAML can resolve native links. DQL's separate resolver does
not currently consult YAML aliases when resolving a destination.

Code fences, inline code and `%%comments%%` are masked for prose links/tags.
Links inside callouts and footnotes are recognized by the native scanner. This
does not establish that callout/list metadata matches Obsidian's metadata cache.

Daily notes use the configured folder/template and support Core Templates date,
time and title expressions plus supported Templater date/title expressions.
A missing configured template fails without creating a note; an existing daily
note remains accessible. Other Templater scripts are not executed. Obsidian's
Daily Notes, Periodic Notes, or Templater settings are not imported automatically.

Native Tasks parsing recognizes checkboxes, configured custom statuses, date
markers (due/done/created/scheduled/start), priorities and recurrence text.
It does not execute Tasks query blocks or import Tasks global filters/status
settings. DQL task metadata is produced by the vendored Dataview importer;
knapper's native task configuration is not the DQL status engine.

## Dataview contract

### Version and evaluation

The embedded implementation is Dataview 0.5.70, pinned at
`5ad0994ff384cbb797de382e7edff2388141b73a`. The upstream parser, expression
functions, query engine and field importer are vendored unchanged under MIT.
Adapters and runtime polyfills are maintained separately.

`TABLE`, `LIST`, `TASK`, `CALENDAR`, source expressions, `WHERE`, `SORT`, `LIMIT`,
`GROUP BY`, `FLATTEN` and upstream functions use that pinned implementation,
including its quirks. Native `knapper query` is a separate language.
The command accepts DQL directly; it does not automatically find/evaluate every
`dataview` fence in a note. No DOM, live refresh or DataviewJS is provided.

### Inputs and index

- DQL pages are scoped `.md`/`.markdown` files. Org and MDX are not DQL pages.
- YAML and inline fields are supplied to the upstream importer. Duplicate fields,
  typed links, dates, durations, nulls and list/task fields follow that importer.
- CommonMark section/list structure plus knapper's link/tag scanner approximate
  Obsidian metadata. Custom-status tasks and ordinary nested lists have tests;
  complex callouts, quoted lists and multiline list ranges remain partial.
- DQL link resolution tries vault-relative and origin-relative paths, optional
  Markdown extensions, then suffix matches ranked by common ancestor, path
  length and lexical order. Suffix candidates are indexed without changing that
  ranking. This is an approximation of Obsidian's destination resolution.
- Ambiguous names, case differences, aliases and special Markdown destinations
  require further actual-Obsidian fixtures. Wrong destinations also affect
  dereferenced fields and incoming/outgoing links.
- `file.ctime`/`mtime`/`size` come from the filesystem. Where creation time is
  unavailable, modification time is used. Copy/sync can change these values.
- `file.starred` follows the saved native bookmark data and plugin state below.
- CSV sources use scoped Vault CSV files. URL/external-file fetching is unsupported.
- `--origin` supplies the current page for `this` and relative sources. An absent
  origin does not automatically select the note open in Obsidian.

### Output, formatting and resource limits

JSON preserves date, duration and link type information. Links carry path,
subpath, display, embed and link type. DQL's headless results are not the DOM
returned by a rendered Obsidian view. Text tables use Markdown; text TASK output
currently uses structured JSON. Partial row errors are reported in diagnostics
and stderr; an all-row evaluation failure fails the command.

`--timezone` accepts an IANA zone and defaults to the host's zone. Bundled Intl
polyfills include English/Japanese locales and timezone data; arbitrary Obsidian
UI locales and host-ICU timezone history are not reproduced.

The default JS heap cap is 256 MiB and the engine deadline is 30 seconds.
`--memory-limit-mib` accepts 16–4096; `--timeout-seconds` accepts 1–600. The deadline
includes JS index construction and evaluation but excludes the preceding Rust
snapshot construction. The heap cap is not a cap on total process memory.
Large vaults can require higher explicit limits; whole-vault import performance
is still an improvement target.

### Evidence and its limits

1. All 18 upstream suites / 395 unchanged test cases pass under Node/Jest and the
   embedded QuickJS runtime. They primarily establish parser/expression behavior.
2. Twenty synthetic end-to-end tests cover the metadata adapter, sources,
   dates/DST, task structure, grouping, errors, Japanese text, native bookmarks and standalone use.
3. Ten full DQL queries are compared with a Node-native-Intl reference. Both
   runtimes use the same adapter, so this does not test Obsidian metadata parity.
4. On 2026-10-03, actual Obsidian 1.13.7 + existing Dataview 0.5.68 was compared
   with knapper's 0.5.70 using three daily-note snapshots and their source notes.
   Six queries agreed on per-account counts, dates, links, aggregates, arithmetic
   checks, filtering, sorting and limit. Private inputs/results remain outside
   this repository. This is limited manual evidence, not an automated whole-plugin
   compatibility claim, and the two plugin versions differed.

Actual-Obsidian checks should pin app/plugin versions, use an isolated profile,
load identical files and relevant settings, wait for both indexes, and compare
ordered typed values with the same origin/timezone. Fixtures committed here must
be synthetic. Include absence, malformed input and ambiguous-resolution cases
when adding an adapter.

## Core Bookmarks contract

Input is `.obsidian/bookmarks.json`. `--obsidian-config-dir` selects an alternate
Vault-relative configuration directory; absolute paths, parent traversal and
settings symlinks escaping the Vault are rejected. The adapter reads only this
file and the matching `core-plugins.json`, without indexing config files as pages
or executing the plugin. Markdown bookmark lists are not an input.

[Obsidian's bookmark documentation](https://obsidian.md/help/plugins/bookmarks)
explains item types and groups. The pinned Dataview `StarredCache.fetch` in
`vendor/dataview/src/data-index/index.ts` determines file-membership semantics:

- Recursively visit nested `group.items`; only `type: "file"` entries contribute
  their exact stored path. File paths are not reinterpreted as aliases.
- Heading/block bookmarks represented by file entries star that file. Duplicate
  entries do not change the boolean.
- Folder, search, graph and URL entries do not star other files implicitly.
- Excluded files do not become DQL pages merely because they are bookmarked.
- Queries do not write/reorder bookmarks or change plugin state.

The settings-file root must have an `items` array. Group child arrays and file
paths are validated; unknown item types are ignored rather than treated as files.
Enabled malformed input fails with a clear error. A missing bookmark file yields
an empty set. A disabled plugin yields an empty set without parsing bookmark data.

Plugin state is read from `core-plugins.json` when bookmarks exist:

| Saved state | Bookmark behavior |
|---|---|
| No core-plugin state file | Read existing bookmark data |
| Boolean state object | Use `bookmarks`; a missing key uses Bookmarks' default-on state |
| Plugin-ID array | Enabled only when `bookmarks` is listed |
| Invalid JSON/state shape | Report an error |

This models a **cold snapshot of saved state**, not live plugin lifecycle/cache
state. In actual Obsidian 1.13.7 + Dataview 0.5.68, disabling an already-loaded
Bookmarks plugin retained its item list: Dataview continued returning true even
after its starred cache was explicitly refreshed. Starting with the plugin
already disabled returned false, matching knapper. This warm-disable difference
is explicit; compatibility does not include reproducing transient retained data.
Legacy migration-file state is not imported.

Four end-to-end tests cover nested groups, subpaths, duplicate/irrelevant items,
exclusions, both plugin-state formats/defaults, missing/malformed data, custom
config directories and escaped symlinks. The synthetic
[actual-Obsidian fixture](tests/fixtures/obsidian-bookmarks/README.md) and
[recorded comparison](tests/fixtures/obsidian-bookmarks/reference-results.json)
cover table/list queries with Bookmarks enabled and disabled at startup. All four
results agree. The warm-disable observation is recorded separately as a known
difference. These comparisons used Dataview 0.5.68 versus the embedded 0.5.70;
same-version lifecycle comparisons remain a future verification improvement.

## Adding a plugin or improving a feature

For each addition, update a registry row and a feature section with:

1. Plugin ID, tested version, input files/settings and enablement conditions.
2. Exact supported syntax and resulting CLI fields/actions. Keep plugin language
   compatibility separate from data-format compatibility.
3. Default/override precedence, missing/malformed-input behavior, read/write scope,
   and differences from native Obsidian behavior.
4. Synthetic positive/negative fixtures; relevant upstream tests when available;
   and a same-version actual-Obsidian comparison for metadata-dependent claims.
5. A clear implemented/partial/unsupported/planned status, known gaps and the
   evidence needed to close each gap.

Before promoting a feature, update tests, this specification and user-facing
entry points together. An unsupported plugin is left as ordinary note content;
recognizing its installation is not executing it or claiming compatibility.

## Next compatibility work

- Same-version actual-Obsidian bookmark checks and additional saved-state edge cases.
- Actual-Obsidian fixtures for ambiguous/case/alias link resolution and complex
  task/list/callout metadata.
- Explicitly scoped configuration import for requested plugins, beyond the
  existing bookmark adapter; no automatic arbitrary-plugin execution.
- Faster whole-vault snapshots/indexing while preserving query and link behavior.
- CI automation of same-version actual-Obsidian differential tests where practical.
