# Extensible PKM compatibility vault

Synthetic fixtures inspired by Dataview's official `test-vault` (books, tasks,
blog/calendar examples), pinned at `5ad0994ff384cbb797de382e7edff2388141b73a`:
https://github.com/blacksmithgu/obsidian-dataview/tree/5ad0994ff384cbb797de382e7edff2388141b73a/test-vault

All people/books/counts here are invented. No private daily notes, book excerpts,
plugin binaries, credentials or workspace/profile caches belong in this tree.
Original upstream examples are MIT; their notice is in `UPSTREAM-LICENSE.txt`.
The fixture notes are newly written using those example patterns.

## Cases and extension contract

`cases.json` is the machine-readable case registry. Each case has a stable ID,
`spec.md` feature ID, DQL query and explicit expected results. The initial seven
cases cover reading lists, nested/custom-status tasks, priorities/due dates,
Japanese text, daily aggregates, links and native bookmarks. TASK comparisons
project only `taskFields`; they do not establish every task position/link field.
Dates are fixed and queries avoid `today` so expectations do not expire.

Rust `extensible_pkm_vault_cases_match_committed_expectations` copies this vault
and checks every case in ordinary CI. `pkm_edits_are_visible_to_fresh_queries`
checks task completion and note create/delete. The live CLI runner uses the same
case manifest. For every new plugin/data behavior or discovered discrepancy:

1. Add the smallest realistic note/setting reproducing it here (or a focused
   sibling fixture when isolating malformed settings/lifecycle states matters).
2. Add a case with independently reviewed expectations and a `spec.md` feature ID.
3. Compare with actual Obsidian App + official CLI and record app/plugin versions,
   supported fields and any intentional differences. Never generate the answer
   key solely from knapper's output. Preserve upstream provenance/notices.
4. Keep the synthetic expected results in CI and update spec/docs together.
   Larger public corpora remain separate, version-pinned inputs.

## Run once or compare live edits

Offline baseline (no Obsidian required):

```sh
python3 scripts/verify-pkm-vault.py --knapper /absolute/path/knapper \
  --vault /absolute/path/to/tests/fixtures/pkm --output /tmp/pkm-baseline.json
```

For live comparisons, **copy** this folder to a disposable vault and open it with
an isolated App profile. Install/enable the chosen Dataview version there, enable
the official CLI in that profile and wait for indexing. Keep the App timezone
at Asia/Tokyo to match the manifest. Set `--obsidian-cli` to its bundled binary:

```sh
python3 scripts/verify-pkm-vault.py --knapper /absolute/path/knapper \
  --vault /tmp/pkm-live \
  --obsidian-cli /path/Obsidian.app/Contents/MacOS/obsidian-cli \
  --output /tmp/pkm-live-result.json
```

Add `--watch` to poll the same queries while editing the disposable copy.
The preview is `pass N: 7 cases matched`; a persistent mismatch exits nonzero
and saves both results. `--interval 5` controls polling and `--iterations 2`
bounds a trial. Ctrl-C stops an unbounded local watch. No scheduler or background
service is installed. Watch checks App/knapper equality rather than demanding the
original baseline after an intentional edit; `baselineEqual` is still recorded.
Transient index differences are retried for up to ten samples at 0.5s intervals;
this is a convergence check, not proof that App refresh is instantaneous.
An App failure, unavailable plugin or wrong vault/timezone is an error.

Both runners read notes/settings; they never perform mutations. Make changes in
the disposable App or via its CLI, then stop the App and discard that copy/profile.
Only sanitized synthetic results and versions belong in committed evidence.
`reference-results.json` records the actual baseline comparison; CI baseline
replay is distinct from executing a desktop App.

## Community-plugin cases

`PluginTasks/Plugin.md` and `tasks-plugin-cases.json` add six Tasks 8.4.0 cases.
`linter-cases.json` adds six Linter 1.33.0 before/after cases (also second-pass
results); `quickadd-cases.json` adds six QuickAdd 2.30.0 Capture cases. Their inputs
are synthetic and their expected outputs were compared with Obsidian 1.13.7 using
the official CLI. Rust CI replays the recorded results and verifies preview/apply,
creation and write-scope behavior. `cases.json` registers these three suites and the known gap via `pluginSuites` /
`pluginKnownGaps`. Their query languages are kept separate from DQL cases;
`verify-pkm-vault.py` still checks the original seven DQL cases.

To repeat the 18 live plugin comparisons, copy this fixture, install/enable those
exact versions in the disposable vault, and open it under an isolated profile:

```sh
python3 scripts/verify-plugin-vault.py --knapper /absolute/path/knapper \
  --obsidian-cli /path/Obsidian.app/Contents/MacOS/obsidian-cli \
  --vault /tmp/pkm-live --output /tmp/plugin-comparison.json --allow-fixture-writes
```

The script prepares synthetic PluginLinter/PluginCapture files and temporarily
sets a Capture Choice in memory (restored afterwards). It refuses this committed
fixture as a write target. It does not install plugins or change a regular Vault.
Version mismatch, wrong vault, plugin absence and comparison differences fail.

`PluginTasks/Collation.md` and `tasks-plugin-known-gaps.json` separately record an
observed numeric text-order difference: App sorts item2 before item10; QuickJS
currently sorts item10 first. This is a known gap, not one of the 18 matching cases.
The live runner reports it separately and requires both recorded orders to agree.
