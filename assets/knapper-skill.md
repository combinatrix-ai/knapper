---
name: knapper
description: Headless CLI for a directory of markdown notes. Use when creating daily notes, querying links and backlinks, reading/writing frontmatter, managing tasks, or renaming/moving notes without breaking links - especially where no note-taking app is running. Triggers on: daily note, backlinks, wikilinks, frontmatter, tasks, vault, knowledge base, markdown notes, Obsidian, org-mode.
---

# knapper

One binary that reads and writes a directory of markdown notes directly -- an
Obsidian vault, a Foam or Dendron workspace, a Zettelkasten, an org-roam
directory, or any folder of `.md` files that has grown links. Nothing needs to
be running, and nothing here touches the network.

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
what they did; `rename` and `move` also accept `--format json`.

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
```

`--no-backlinks` skips building the link graph, which is the expensive part on
a large vault.

## Links

```bash
knapper links FILE                  # outgoing, both syntaxes
knapper backlinks FILE              # what references this file
knapper backlinks FILE -A 2 -B 2    # with surrounding lines, like grep
knapper orphans                     # nothing links here
knapper hubs --limit 10             # most linked-to
knapper broken-links                # links pointing nowhere
```

`orphans` hides `Templates/` and dotfolders; `--include-special` shows them.

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

## External references

A note can point at a value it does not contain -- an address, a key, a token
-- with an ordinary markdown link:

```markdown
[日本橋小舟町の住所](knapper://personal/address.nihonbashi_kobunacho)
```

The shape is `knapper://<provider>/<locator>`. The provider is a name the user
chose, not a tool; the locator is opaque.

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
**4** when the command fails. It may prompt the user interactively -- for a
PIN, a passphrase or a hardware key -- so run it in the foreground.

There is no default timeout. `--timeout SECS` bounds the whole resolve,
including the wait for provider stdout to close; a provider remains responsible
for any further processes it starts.

Provider commands live in `$XDG_CONFIG_HOME/knapper/providers.yaml`
(`~/.config/knapper/providers.yaml` by default), never in the vault:

```bash
knapper provider list
knapper provider set personal -- op read 'op://Knapper/{locator}/value'
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
```

## Keeping knapper current

```bash
knapper self-update --check
knapper self-update
```

This is the only command that opens a network connection.

## Notes that matter in practice

- A malformed YAML header never aborts a whole-vault scan; that note is read
  without its frontmatter.
- Links inside code fences, inline code and `%%comments%%` are not links, and
  do not appear in the graph.
- `knapper.config.md` can `exclude:` whole subtrees (imported archives,
  generated logs). Every command honours it.
- `.org` files are read by a dedicated parser, including `[[id:...]]` and
  global `[[*Heading]]` links.
