---
name: knapper
description: Headless CLI for a directory of markdown notes. Use when creating daily notes, querying links and backlinks, reading/writing frontmatter, managing tasks, renaming/moving notes without breaking links, or resolving knapper:// external references through a configured provider - especially where no note-taking app is running. Triggers on: daily note, backlinks, wikilinks, frontmatter, tasks, vault, knowledge base, markdown notes, Obsidian, org-mode, knapper://, external reference, secret reference, resolve.
---

# knapper

One binary that reads and writes a directory of markdown notes directly -- an
Obsidian vault, a Foam or Dendron workspace, a Zettelkasten, an org-roam
directory, or any folder of `.md` files that has grown links. Nothing needs to
be running. knapper itself touches the network only for `self-update`; a
provider command configured for `resolve` may open its own connection.

Use it for the operations that need to understand a vault's *structure*, which
plain shell tools cannot do: the link graph, link-preserving renames, tasks,
frontmatter.

**Do not use it for full-text search.** knapper ships none. Use `rg`, and
compose the two:

```bash
knapper orphans --format paths | xargs rg -l "TODO"
```

If Obsidian itself is running on this machine, prefer the official `obsidian`
CLI (https://help.obsidian.md/cli) for anything it covers -- Bases, Sync,
Publish, workspaces. knapper is for everywhere that is not true.

## Setup

knapper walks up from the working directory to find the nearest
`knapper.config.md`, so commands work from any subdirectory once one exists.

```bash
cd ~/your-vault
knapper init          # writes knapper.config.md; defaults are sane
```

## Output formats

Query commands take `-f/--format`:

- `text` (default) — for a human
- `json` — parse this, not the text
- `paths` — bare paths, for piping

Mutating commands (`rename`, `move`, `frontmatter set`, `tasks new`) report
what they did; `rename` and `move` also accept `--format json` and
`--dry-run`.

**Always pass `--format json` when you intend to parse the result.**

## Finding notes

`query` is the general form; `orphans`, `hubs` and `broken-links` are presets
over it.

```bash
knapper query --where status=open --from Questions --sort inlinks:desc
knapper query --where inlinks=0                  # what orphans does
knapper query --where broken>0 --field broken    # what broken-links does
knapper query --sort inlinks:desc --limit 10     # what hubs does
knapper fields                                   # what this vault can be filtered on
```

Frontmatter keys, Dataview inline fields (`[cost:: 42]`) and computed fields
share one namespace, so `status=open`, `cost>40` and `inlinks=0` all work the
same way. Operators: `=` `!=` `>` `<` `>=` `<=` `~` (contains), a bare name for
"has this field", `!name` for "does not".

Computed fields: `path` `title` `ext` `inlinks` `outlinks` `broken` `tags`
`words` `chars` `lines` `modified` `age`.

## Reading one note

`context` aggregates what would otherwise take five calls -- content, links,
backlinks, tags, inline fields, headings, tasks, stats:

```bash
knapper context "Notes/Lit Review.md" --format json
```

Trim it when the whole note is not needed:

```bash
knapper context FILE --no-content --no-backlinks   # structure only
knapper context FILE --max-content 2000            # truncated, marked as such
knapper context FILE --line 120                   # focused context, default -B 3 -A 3
knapper context FILE --line 120 --section         # smallest enclosing Markdown section
knapper context FILE --line 120 --outline-depth 2 # include nested outline entries
```

`--no-backlinks` skips building the link graph, which is the expensive part on
a large vault.

Focused `context --line N` addresses a physical 1-based line and returns a
line-numbered excerpt, the complete heading breadcrumb, and a first-layer
document map. `-B/--before` and `-A/--after` override the default three-line
window. `--section` replaces the window with the smallest enclosing Markdown
section and cannot be combined with explicit `-A` or `-B`; `--outline-depth 0`
hides the map. H1 children form the first layer when an H1 title exists,
otherwise the shallowest heading level does. Maps are compacted above twelve
entries and report their omission counts. `--no-content` keeps focus metadata
but omits the excerpt text, while `--max-content` truncates around the focused
line. Focused mode rejects org-mode notes until an org outline contract exists.

## Links

```bash
knapper links FILE                  # outgoing, both syntaxes
knapper backlinks FILE              # what references this file
knapper backlinks FILE -A 2 -B 2    # with surrounding lines, like grep
knapper orphans                     # nothing links here
knapper hubs --limit 10             # most linked-to
knapper broken-links                # links pointing nowhere
knapper broken-links --format json  # one record per occurrence, with position
```

`orphans` hides `Templates/` and dotfolders; `--include-special` shows them.

`broken-links --format json` is occurrence-oriented, not a per-file summary:
each record carries `source`, `line`, `column` (1-based, in characters, and
`null` for org), `syntax`, `raw` (the link as written), `raw_target`, `target`
(what the resolver was asked), `reason`, `status` and `candidates`. It reads
the same scan as `repair-links`, so the two cannot disagree.

## Hard links and soft tags

Two kinds of reference, and they mean different things:

- `[[X]]` is a **hard note reference**. It names a note, it is an edge in the
  graph, and a missing target is a broken link.
- `#X` is a **soft topic reference**. It labels a note with a subject, needs
  no note to exist, is never broken, and is never an orphan or a hub.

A `#tag` argument makes the soft one navigable, as a *virtual* subject:

```bash
knapper backlinks '#COO採用'                 # notes and lines carrying it
knapper backlinks '#COO採用' -A 2 --format json
knapper context '#COO採用' --format json     # the whole topic, as one object
```

Quote it: an unquoted `#` starts a comment in most shells.

- A leading `#` is the **only** thing that selects a tag. Everything else is a
  note, so nothing changes for ordinary arguments.
- A tag covers the tags **nested** under it: `#work` finds `#work/hiring`.
  Every result names the tag it actually matched.
- Matching is otherwise **exact and case-sensitive**, with no Unicode
  normalisation — the same rule `tags --find` follows.
- Occurrences come from prose (`where: "inline"`) and from metadata
  (`where: "declared"`: YAML `tags:`, org `#+filetags:` and heading tags).
  Code fences, inline spans and `%%comments%%` hold no occurrences.
- Results are sorted by path then line, so the JSON is stable between runs.
- `context '#tag'` returns `kind: "tag"` with `notes`, `occurrences`,
  `nested_tags`, `tasks` and `stats`, and no `path` — nothing pretends a note
  exists. `--no-content` drops the line text, `--max-content` truncates it and
  `--no-tasks` drops the tasks; `--no-backlinks` has nothing to skip.

### Demoting a wikilink to a tag

For the vault that has been writing `[[COO採用]]` for years as a label rather
than as a claim that such a note exists:

```bash
knapper demote "COO採用" --dry-run --format json
knapper demote "COO採用"
```

Only the **exact** form `[[COO採用]]` becomes `#COO採用`. Everything a tag
cannot hold is reported under `skipped`, with a file, a line and a reason, and
left exactly as it was: `alias`, `anchor`, `embed`, `path-qualified`,
`adjacent text` (`見た[[X]]の` would produce a different tag), `frontmatter`
and `org-mode`. Code and comments are not references and are neither rewritten
nor reported.

- `--dry-run` reports every proposed edit and writes nothing.
- A target that names a **note that exists** is refused: demoting it would
  drop a real reference. `--allow-existing-note` does it anyway.
- A target that cannot be spelled as a tag (`Daily Tasks`) is refused before
  anything is read; `--tag work/daily` names the tag to write instead.
- A path-qualified target (`Archives/X`) is refused: `demote` names a bare
  target. Use `--tag a/b` to write a nested tag.

Promotion the other way (`#X` back to `[[X]]`) is deliberately not
implemented: it would have to invent a note, choose where it lives, and turn a
label into a promise. Create the note and use `rename`.

## Renaming without breaking links

The reason to reach for knapper at all. Every inbound link is rewritten, in
both syntaxes, preserving link text, folder prefixes, `#anchors` and
percent-encoding; external URLs and image embeds are left alone.

```bash
knapper rename "Thesis" "Dissertation"
knapper move "note.md" "Archive/"
knapper rename OLD NEW --dry-run    # preview first
```

Prefer `--dry-run` before a rename you are not certain about.

### Moving a directory

`move` takes a directory too. The subtree moves as it stands — notes,
attachments, sidecars, dotfiles, nested folders — and every link that
*resolves* into it follows.

```bash
knapper move "Projects/Thesis" "Archive/" --dry-run --format json
knapper move "Projects/Thesis" "Archive/" --format json
```

- The source is an **exact** vault-relative directory path, trailing slash
  optional. There is no name matching: `move Thesis Archive/` will not find
  `Projects/Thesis`.
- The result is always `DEST/<the directory's own name>`. It does not rename,
  and it refuses rather than merging into an existing directory.
- "Inbound" means resolution, not text. A bare `[[README]]` that meant some
  other README is left alone; one that meant the moved README is qualified so
  it still resolves. Relative links out of the subtree are recomputed; links
  within it are left alone when the structure keeps them working. Attachment
  and image links follow their files. Prose, code, `%%comments%%`, external
  URLs and `knapper://` references are never touched.
- JSON reports `kind`, `old_path`, `new_path`, `entries`, `notes`,
  `files_updated`, `links_updated`, `unsupported_links` and `warnings`.
  `--dry-run` adds `moves` (every file's old and new path) and `edits` (every
  link change, with line, before and after) — enough to act on without
  reading the vault again — and writes nothing.
- Both ends must be inside the real vault tree. A source or destination that
  reaches its directory through a **symlink** is refused, however
  vault-relative the path looks: following it would move the directory out of
  the vault and leave every rewritten link pointing nowhere. A symlinked
  *note* is different and is followed: rewriting a link inside one updates
  the file it points at and leaves the symlink a symlink.
- Rewritten inline links are percent-encoded for the path they now have. A
  directory called `Guide (v2)` gives `[a](Archive/Guide%20%28v2%29/Note.md)`,
  since an unencoded `)` would end the link early; `%`, `"`, `<`, `>` and `#`
  are encoded for the same reason. Wikilinks are not URL-like and are left
  plain; a `<...>` destination keeps its angle brackets.
- knapper does not rewrite org-mode links. If an inbound `.org` link points
  into the directory, the move **stops and changes nothing**, listing them
  under `unsupported_links`; `--allow-broken-org-links` proceeds anyway and
  still reports them. Run `--dry-run` first to see them.

Use `--dry-run` before every directory move: it is the cheap way to see
exactly which links a move would touch.

## Repairing links that broke somewhere else

`rename` and `move` keep links intact through a refactor knapper performs.
`repair-links` is the other half: the ones that broke while knapper was not
looking — a folder reorganised elsewhere, an exporter's stale path, an
encoding that survived a round trip.

```bash
knapper repair-links --dry-run                  # the plan, as text
knapper repair-links --dry-run --format json    # the plan, machine-readable
```

**V1 plans and nothing else.** `--dry-run` is required; omitting it exits 2
before the vault is read. There is no `--apply`, so never tell a user this
command fixed anything — hand them the plan.

Every occurrence is `safe`, `ambiguous` or `unresolved`. Only two bases are
ever safe, and both are structural:

- `unique-path-suffix` — the tail of the path is still exactly right and only
  the leading directories are stale, and exactly one file ends that way.
  Leading `../` counts as part of the stale prefix.
- `percent-decoding` — decoding the target yields the link that was meant,
  *and* the decoded text names a path that exists.

Both are checked by re-resolving the link knapper would write, as a path.
Decoding is not a doorway to anything looser: knapper resolves a bare name by
stem and by alias when it reads a vault, and neither is evidence for an edit.
`[[Docs/My%20Note]]` is safe; `[[My%20Note]]` reaches the same file by
filename and is only a suggestion.

Everything else is a suggestion at most, and knapper will not act on it:

- `basename` — the filename matches and the directory does not. A
  resemblance, not a path.
- An alias — a name the author gave a note is not a location.
- `path-suffix` with more than one hit — both destinations are listed and
  neither is chosen.
- A renamed concept (`[[Roam]]` beside `RoamResearch.md`) is `unresolved`.
  Do not "fix" it on knapper's behalf; ask the user.
- `[[2026-07-04]]` (`missing-date`) and `[[12]]` (`numeric-label`) name no
  note, so nothing is invented — no note, no tag.
- org links are reported with a line and never with a plan.

A link that already resolves is never a repair, including a valid
note-relative link; excluded notes and non-note leaves *are* valid
destinations, exactly as they are for the resolver.

JSON reports `kind`, `dry_run`, `applied`, `summary`
(`files`/`occurrences`/`safe`/`ambiguous`/`unresolved`) and `occurrences`.
Each occurrence adds `candidates` (`path`, `basis`, `matched`) and, when it is
safe, an `edit` with `byte_start`, `byte_end`, `before`, `after`,
`target_before`, `target_after` and `resolves_to` — enough to carry the repair
out without scanning again. `edit` is `null` for everything that is not safe.
Occurrences are ordered by source, then line, then column.

## Tasks

Plain markdown checkboxes. The Obsidian Tasks emoji convention (`📅` due,
`⏳` scheduled, `🛫` start, `✅` done, `➕` created, `🔁` recurring, `⏫🔼🔽`
priority) is read and written but never required.

```bash
knapper tasks                                   # open tasks
knapper tasks --overdue
knapper tasks --due-from 2026-08-01 --due-to 2026-08-31
knapper tasks --status wip --tag work
knapper tasks --breadcrumbs                     # with the heading path
knapper tasks --group date                      # or --group file
knapper tasks --exclude Archive/
```

Dates must be `YYYY-MM-DD`. Relative values such as `today` or `+3d` are
rejected rather than guessed at, with exit status 2.

Changing a task:

```bash
knapper tasks new "write the README" --due 2026-08-01
knapper tasks done "write the README"    # exact match; --partial for substring
knapper tasks wip TEXT
knapper tasks cancel TEXT
knapper tasks set STATUS TEXT            # config-defined statuses too
```

An ambiguous match is refused, not guessed. Narrow it with `--file`.

Built-in statuses: `open` `[ ]`, `wip` `[/]`, `done` `[x]`, `cancel` `[-]`.
`knapper.config.md` can override their markers or add new ones.

## Daily notes

```bash
knapper daily                # today, created from the template if missing
knapper daily yesterday
knapper daily 2026-01-15
knapper daily --path-only    # just the path, for scripting
```

Templates expand on creation: Obsidian core (`{{date}}`, `{{time}}`,
`{{title}}`) and Templater date expressions (`<% tp.date.now("YYYY-MM-DD", -1) %>`,
`<% tp.date.yesterday() %>`, `<% tp.file.title %>`).

## Frontmatter and tags

```bash
knapper frontmatter get FILE [KEY]
knapper frontmatter set FILE status done
knapper frontmatter delete FILE KEY
knapper tags                 # every tag with counts
knapper tags --find cli      # files carrying a tag
```

Tags are Unicode-aware and nest: `#日本語` and `#parent/child` both work.
`tags --find` matches one tag exactly; `backlinks '#tag'` covers nested tags
too and reports the line each occurrence is on.

## External references

A note can point at a value it does not contain -- an address, a key, a token
-- with an ordinary markdown link:

```markdown
[日本橋小舟町の住所](knapper://personal/address.nihonbashi_kobunacho)
```

The shape is `knapper://<provider>/<locator>`. The provider is a lowercase
name the user chose, not a tool; it uses ASCII letters, digits, `_`, and `-`.
The locator is opaque but its alphabet is fixed: ASCII letters, digits, `.`,
`_`, `-`, `/`, with a maximum of 128 characters and no empty, `.`, or `..`
segments. A destination outside that grammar is silently not a reference; if
an expected link is missing from `refs`, check its spelling first.

Find them:

```bash
knapper refs FILE --format json
knapper refs --provider personal --format json
knapper refs --format paths
```

Results include `path`, `line`, `column`, `uri`, `provider`, `locator` and
`label`. References inside fenced code, inline code and `%%comments%%` are
ignored, and `knapper context FILE --format json` reports the same under
`references`. A `knapper://` link is external: it is not part of the link
graph and `rename` and `move` do not touch it.

Read one value:

```bash
knapper resolve "knapper://personal/address.nihonbashi_kobunacho"
knapper resolve "knapper://work/tokens/ci.deploy" --dry-run   # print the argv, run nothing
knapper resolve "knapper://work/tokens/ci.deploy" --timeout 30
```

`resolve` runs the command the user configured for that provider and prints
its stdout, with one trailing newline removed and no newline added. It exits
**2** on a malformed reference, **3** when the provider is not configured, and
**4** when the command would not run, failed, timed out, or returned nothing
usable. Values must be UTF-8 text of at most 1 MiB. It may prompt the user
interactively -- for a PIN, a passphrase or a hardware key -- so run it in the
foreground.

There is no default timeout. `--timeout SECS` bounds the whole resolve,
including the wait for provider stdout to close; a provider remains responsible
for any further processes it starts.

Provider commands live in `$XDG_CONFIG_HOME/knapper/providers.yaml`
(`~/.config/knapper/providers.yaml` by default), never in the vault:

```bash
knapper provider list
knapper provider set personal -- op read 'op://Knapper/{locator}/value'
knapper provider set family -- pass show 'knapper/{locator}'
knapper provider remove personal
```

`command` is argv, not a shell line; every `{locator}` in it is replaced.
knapper is provider-agnostic and knows nothing about any password manager --
`op` above is only an example, and `pass`, `security`, `bw`, `vault` or a
local script work the same way.

Rules to follow: `knapper refs` shows only *where* a reference is, never what
it is worth -- do not claim a value from it. Never write a resolved value into
a note, a commit, a log or a file; knapper itself does not cache or store one.
Never propose putting provider commands in `knapper.config.md`; the vault is
not a place executable configuration can come from.

## Vault health

```bash
knapper lint                 # broken links, orphans, duplicates, stubs, missing frontmatter
knapper lint --check broken-links --format json
knapper repair-links --dry-run   # which of those broken links could be repaired
```

## Keeping knapper current

```bash
knapper self-update --check
knapper self-update
```

This is the only command where knapper itself opens a network connection; a
provider command configured for `resolve` may open its own.

## Notes that matter in practice

- A malformed YAML header never aborts a whole-vault scan; that note is read
  without its frontmatter.
- Links inside code fences, inline code and `%%comments%%` are not links, and
  do not appear in the graph. The same goes for tags.
- A tag is never a graph node. `orphans`, `hubs` and `broken-links` do not
  change shape because a vault uses tags, and a `#tag` is never broken.
- `knapper.config.md` can `exclude:` whole subtrees (imported archives,
  generated logs). Every command honours it.
- `knapper.config.md` can also `ignore_links:` link targets that are meant to
  stay unresolved. `lint`, `broken-links` and `query --where broken>0` stop
  reporting them. An entry matches a whole target, case-insensitively; it is
  never a substring, and a path-qualified link needs its path written out.
- `.org` files are read by a dedicated parser, including `[[id:...]]` and
  global `[[*Heading]]` links.
- `repair-links` never writes, and never guesses. A broken link it leaves
  `unresolved` is a question for the user, not an invitation to invent a note
  or demote it to a tag. `demote` is for a label that was never a reference;
  it is not link repair.
