# Pinned community plugin engines

Normal Rust builds embed generated bundles and need no Node at runtime.
Development uses `npm ci --prefix plugins --ignore-scripts` and
`npm run build --prefix plugins`. CI regenerates bundles/notices and checks drift.
`knapper licenses` prints the embedded notices outside any vault.

Tasks 8.4.0's query/parser source is unchanged in `vendor/tasks`; adapters live
here and in `src/tasks_query.rs`. Four selected unchanged upstream suites (40
cases) run in QuickJS; this is not the full Tasks suite. The six recorded
App/official-CLI comparisons use the same Tasks version. Query instructions are
explicitly gated to the subset in spec.md; no custom JS or host API execution.
The runtime has no filesystem/network/process callbacks, a 256 MiB JS heap and
30-second engine deadline. `runtime.ts` maps initialization scheduling to
microtasks; it does not implement general elapsed-time timers.

Linter 1.33.0 has three selected unchanged rules and protected-range algorithms;
its UI option controls are replaced with headless metadata. QuickAdd 2.30.0 uses
unchanged capture/user-text/body-insertion helpers and a content-only Obsidian
frontmatter adapter. Their full plugin runtimes and installed settings are not
executed. The selected suites total Tasks 40 + Linter 54 + QuickAdd 40 = 134 cases.
See spec.md for default options, supported tokens, write scope and known gaps.
