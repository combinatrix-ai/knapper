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
