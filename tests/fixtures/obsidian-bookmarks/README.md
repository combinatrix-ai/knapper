# Synthetic Obsidian bookmark fixture

Open a temporary copy of this vault in Obsidian with Core Bookmarks and
Dataview enabled. Do not add plugin binaries or generated workspace/profile
files to this fixture. Compare these DQL queries with `knapper dql`:

```sql
TABLE WITHOUT ID file.path, file.starred
WHERE file.name != "README" AND file.folder != "Excluded"
SORT file.path
```

Expected rows: `["A.md", true]`, `["B.md", true]`, `["C.md", false]`.
A heading/block bookmark marks the file; duplicate and nested-group entries are
handled. Folder/search/graph/URL bookmarks do not imply starred files.

```sql
LIST WITHOUT ID file.path
WHERE file.starred AND file.folder != "Excluded"
SORT file.path
```

Expected values: `A.md`, `B.md`. The explicit exclusion in the Obsidian queries
matches knapper's `exclude: [Excluded]`; installing knapper does not make Obsidian
read knapper.yaml. Disable Core Bookmarks, restart Obsidian, and repeat: all booleans should be
false and the starred-only query should return no rows. A live disable may retain
loaded items in Dataview; see spec.md for that explicit difference.

## App + official CLI differential check

Copy this fixture to a disposable vault; install the tested Dataview version
there and open it in an isolated Obsidian profile (`--user-data-dir`). Enable
General → Advanced → Command line interface in that profile. No global PATH
registration is needed when invoking the app's bundled CLI binary directly.
The runner targets the vault name and checks its absolute path, validates plugin
state, waits for the fixture's five indexed pages, and compares the two queries
against knapper and the recorded reference. Use Python 3 and absolute binaries:

```sh
python3 scripts/verify-obsidian-cli.py \
  --obsidian-cli /path/Obsidian.app/Contents/MacOS/obsidian-cli \
  --knapper /path/knapper \
  --vault /path/disposable-vault \
  --state enabled --output /tmp/bookmarks-enabled.json
```

Then run `obsidian-cli vault=disposable-vault plugin:disable id=bookmarks filter=core`,
fully quit that isolated App, restart it with the same isolated profile, and
repeat with `--state cold-start-disabled` and a different output file. The runner
is read-only; state changes and full restart are explicit setup steps.
`cli-reference-results.json` records all four successful comparisons on
2026-10-03. The installed Dataview version was 0.5.68, embedded version 0.5.70;
this does not replace future same-version validation. Desktop execution remains
an opt-in integration check and requires a running App; ordinary CI uses the
existing recorded-reference regression without starting Obsidian.
