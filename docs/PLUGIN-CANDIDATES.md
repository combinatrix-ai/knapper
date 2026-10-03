# Plugin candidate review

Reviewed 2026-10-03. These are source-inspected candidates, not implemented or
runtime-tested compatibility. The existing embedded Dataview runtime does not
establish that other plugins run unchanged in QuickJS.

## Initial candidates

| Plugin | Inspected version and commit | Root license | Recommended first scope |
|---|---|---|---|
| [Tasks](https://github.com/obsidian-tasks-group/obsidian-tasks) | 8.4.0; `692e965ecbaad197221fae9ddff13f5c5fa6ece6` | MIT | Read-only task parsing and query filters, sorting, grouping and limits |
| [Linter](https://github.com/platers/obsidian-linter) | 1.33.0; `b15df18a182bbbc750209a8913a89469a164d01a` | MIT | Selected deterministic formatting rules with explicit settings and rule order |
| [QuickAdd](https://github.com/chhoumann/quickadd) | 2.30.0; `943649ddb105ee166c52b44222e7178d7632a98a` | MIT | Configured Capture/Template choices with all inputs and context supplied explicitly |

### Tasks

`src/Query/Query.ts` exposes `applyQueryToTasks`; task parsing, recurrence and
serialization live in `src/Task/` and `src/TaskSerializer/`. These provide useful
boundaries for reusing upstream behavior and tests. They still depend on settings,
status registries, scripting contexts and date APIs, including `window.moment`.
Recurrence uses `rrule`. Audit the transitive imports before deciding which
modules can be embedded.

Start with query examples such as `not done`, date filters, sorting/grouping and
limits. Custom JavaScript query functions and rendered UI need separate scope.
Completion and recurrence writes require explicit source-location handling and
mutation fixtures. Freeze clock, timezone, locale, status settings and file context
in comparisons. Reuse upstream Query/Task/serializer tests, then compare a pinned
plugin in the synthetic vault through the actual Obsidian App and official CLI.

### Linter

`src/rules-runner.ts` has `RulesRunner.lintText`, returning a string. Its run options
include file information, settings, locale and an injectable current-time function.
The module also imports `TFile` and `moment` from Obsidian; it is not immediately
standalone. Individual rules use protected ranges and registration machinery, and
rule ordering affects the result.

Start with selected YAML, whitespace and heading rules. A proposed dry-run should
show the resulting diff before an explicit write. File timestamps, paste/editor
operations, custom commands and UI need separate treatment. Check dependency
runtime requirements as well as licenses. Reuse individual rule examples/tests and
compare exact bytes, protected code blocks, frontmatter and idempotence in App
fixtures. This is a proposal, not a new CLI contract.

### QuickAdd

`src/engine/CaptureChoiceEngine.ts` and `TemplateEngine.ts` depend on editor state,
prompts, active views, file operations and integrations. The formatter also uses
Obsidian file types and context-sensitive tokens. Some helpers, such as
`src/formatters/helpers/capturePlacement.ts`, are narrower string operations;
this does not make the full Capture/Template engine pure.

Start with a configured choice and explicit date, path, variables and destination,
then create/append the expected note. Defer interactive prompts, clipboard,
selection/cursor behavior, macros, AI operations and arbitrary Templater scripts.
Use upstream formatter/engine tests where applicable and compare exact generated
notes against a pinned App fixture. Inspect settings defaults and collision/error
behavior before implementing writes.

## License decision before vendoring

All three inspected roots are MIT. Retain their copyright/license notices and
review every bundled dependency and copied asset before redistribution; root
licenses alone are not a complete distribution audit.

knapper currently declares MIT. [Templater's license](https://github.com/SilentVoid13/Templater/blob/master/LICENSE.TXT)
is AGPLv3. Being open source alone is insufficient. For a deeply integrated single
binary containing AGPL-covered code, plan to distribute the combined covered work
under AGPL-compatible terms, retain MIT notices, and provide complete Corresponding
Source, including required build/install material (§§1, 5, 6). A modified version
supporting remote network interaction additionally has the source-offer requirement
in §13. Independent aggregation is different from such integration. This is a
planning interpretation of the license text, not a completed dependency audit.
No license change or Templater vendoring has been performed.

## Further preliminary candidates

These are functional leads, not a popularity ranking or completed source audit.

| Candidate | Root license observed | Potential scope |
|---|---|---|
| [Periodic Notes](https://github.com/liamcain/obsidian-periodic-notes) | MIT | Daily/weekly/monthly paths and templates |
| [Tag Wrangler](https://github.com/pjeby/tag-wrangler) | ISC-style | Tag rename/merge, with body/YAML mutation fixtures |
| [Kanban](https://github.com/community-archive/obsidian-kanban) | GPLv3 | Markdown board format; maintainer/archival status needs review |
| [Omnisearch](https://github.com/scambier/obsidian-omnisearch) | GPLv3 | Search/ranking; OCR/PDF host dependencies separately |
| [Templater](https://github.com/SilentVoid13/Templater) | AGPLv3 | Template evaluation after license and host API decisions |

MarkMind is identified as closed source by the official directory. Smart
Connections uses source-available terms with competitive-use restrictions. Neither
is an unconditional OSS-vendoring candidate.

## Implementation gates

Choose and document a narrow behavior slice; pin the source and dependencies;
audit notices/licenses and host imports; adapt the upstream tests; extend
`tests/fixtures/pkm`; record same-version App/CLI results; then update `spec.md`
with implemented behavior and remaining differences. Candidate inspection has not
run the upstream suites or proved QuickJS execution for these plugins.
