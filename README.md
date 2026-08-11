# 🪨 knapper

**A headless CLI for markdown knowledge bases, built for AI agents to drive.**

knapper reads and writes the note files directly. No app to launch, no daemon,
no server, no index to build, no API keys — one binary, and nothing to install
beside it. knapper itself touches the network only for `self-update`, which
runs only when you ask for it by name. `knapper resolve` can execute a provider
command you configured; whether that command uses the network is up to the
provider you chose. knapper resolves **both** link syntaxes,
`[[wikilinks]]` and `[inline](links.md)`, and reads `.org` files too, so it
works on an Obsidian vault, a Foam or Dendron workspace, a Zettelkasten, an
org-roam directory, or any folder of notes that has grown links.

```bash
knapper backlinks "Projects/thesis.md"   # what references this file?
knapper rename "thesis" "dissertation"   # rename it — every link follows
knapper tasks --overdue --format json    # machine-readable, ready for jq
```

> A *knapper* is someone who strikes obsidian and flint into usable blades.
> This one works a directory of markdown into something an agent can wield.

## Supported note formats

| Ecosystem | Support | What knapper understands |
|---|---|---|
| **Obsidian** | ✅ | Wikilinks, markdown links, properties, aliases, tags, daily notes, and the Tasks, Dataview, and Templater conventions |
| **Foam** | ✅ | Wikilinks and markdown links, YAML frontmatter, and tags |
| **Dendron** | ✅ links | Dot-hierarchy filenames and links with or without `.md`; frontmatter IDs are read but do not resolve links |
| **Zettelkasten** | ✅ | ID-prefixed filenames and bare wikilinks |
| **Plain markdown / SSGs** | ✅ | Markdown links and YAML frontmatter in `.md`, `.markdown`, and `.mdx` files |
| **Logseq** | ⚠️ file-level | Page references, properties, tasks, and dates; block identity is intentionally ignored |
| **Roam exports** | ⚠️ file-level | Page references, macros, and tasks; block identity and attributes are not modelled |
| **org-mode / org-roam** | ⚠️ read support | Links, IDs, global heading links, properties, tags, todos, and planning dates; write commands still emit markdown |

Every claim above is pinned by a fixture test. See
**[the full compatibility matrix](docs/COMPATIBILITY.md)** for exact syntax,
plugin coverage, configuration, and known gaps.

---

## Why this exists

Your notes are markdown on disk. Your agent is in a terminal — often one with
no desktop attached: a cron job, a CI runner, a container, a box you reached
over SSH. knapper gives that agent the operations plain shell tools can't do,
because they require understanding the *structure* of a vault:

- 🔗 **The link graph** — `backlinks`, `links`, `orphans`, `hubs`, `broken-links`
- ✂️ **Link-safe refactors** — `rename` and `move` rewrite every inbound link, in both syntaxes; `move` takes a whole directory
- 🩹 **Link repair planning** — `repair-links --dry-run` finds the links that broke elsewhere and proposes only the repairs the filesystem settles
- ✅ **Tasks** — query and mutate `- [ ]` checkboxes across the whole tree
- 📇 **Frontmatter** — get, set, and delete YAML fields from the shell
- 🩺 **Vault health** — `lint` finds broken links, orphans, duplicate names, stubs, missing frontmatter
- 🧠 **`context FILE`** — everything an LLM needs about one note, in a single call
- 🔐 **External references** — `knapper://` links to values a provider command resolves, kept out of the vault

It is built to survive real vaults: tags are Unicode-aware and nest
(`#日本語` and `#parent/child` both work), and one malformed YAML header never
aborts a whole-vault scan.

## Two tools you might want instead

Honesty first — knapper has two excellent neighbours, and sometimes they are
the right choice.

**If Obsidian is running, use the official
[Obsidian CLI](https://help.obsidian.md/cli)** (shipped with Obsidian 1.12).
It is first-party and talks to the live app, so it reaches things knapper
never will: Bases, Sync, Publish, workspaces, file recovery, plugin
development. It also covers most of knapper's surface. Its one requirement —
its docs state it plainly — is that the app must be running. knapper is for
everywhere that isn't true, and for directories that were never an Obsidian
vault to begin with.

**For full-text search, use [ripgrep](https://github.com/BurntSushi/ripgrep).**
knapper deliberately ships no search and no index — `rg` already won that.
Prose is `rg`'s; structure is knapper's, and the two compose in that order:

```bash
# find candidates by text, then ask knapper about their structure
rg -l "some phrase" Diary | head -3 | xargs -n1 knapper context --format json
```

The line between them is not "which tool do I like" but **what the question
is about**. Markdown structure looks regular enough to grep and is not:

| Tempting | What it misses | Ask instead |
|---|---|---|
| `rg -o '\[\[' -g '*.md'` | `![[embeds]]`, `[[X\|alias]]`, `[[X#heading]]`, `[text](x.md)`; and it counts links inside code fences that aren't links | `knapper links` / `knapper backlinks` |
| `rg -l '^status:' -g '*.md'` | a body line that starts the same way; quoted values, lists, Dataview `status:: open` | `knapper query --where status=open` |
| `rg -l '#project' -g '*.md'` | `#project/sub` nests; a `#` in a URL or heading is not a tag | `knapper backlinks '#project'` |
| `rg '\]\(.*\.md\)'` | which of those targets actually resolve, once basenames, aliases, relative paths and `ignore_links` are taken into account | `knapper broken-links` |
| `rg '^\s*- \[ \]'` | status characters (`- [/]`, `- [-]`), due dates, tags, excluded subtrees; and `--prose-only` drops the checkboxes that live in fenced examples | `knapper tasks --overdue --tag work` |

## Install

Download the binary for your platform from
[Releases](https://github.com/combinatrix-ai/knapper/releases). It is one
file, about 5 MB, with nothing to install alongside it — no interpreter, no
virtualenv, no dependency resolution. That is the point: knapper is for
machines where you would rather not stand up a runtime.

```bash
curl -fsSL https://raw.githubusercontent.com/combinatrix-ai/knapper/main/install.sh | sh
```

That puts the binary in `~/.local/bin`, verifies it against the release's
`SHA256SUMS`, and registers the agent skill with whichever of Claude Code and
Codex it finds. `--bin-dir`, `--version` and `--skill none` change all three;
`--help` lists them.

Or take the archive yourself, from
[Releases](https://github.com/combinatrix-ai/knapper/releases):

```bash
curl -L https://github.com/combinatrix-ai/knapper/releases/latest/download/knapper-aarch64-apple-darwin.tar.gz | tar xz
sudo mv knapper/knapper /usr/local/bin/
```

Linux builds are musl-linked, so they run on Alpine and in a scratch
container.

Because there is no package manager in that story, knapper upgrades itself:

```bash
knapper self-update --check   # is there a newer release?
knapper self-update           # install it
```

It replaces the running binary in place, so it needs write permission where
knapper lives — under `/usr/local/bin` that means `sudo`. This is the only
command that opens a network connection.

<details>
<summary>From source</summary>

```bash
cargo install --git https://github.com/combinatrix-ai/knapper knapper
```

</details>

## Quick start

```bash
cd ~/your-notes
knapper init             # writes knapper.config.md — defaults are sane
knapper lint             # how healthy is this vault?
knapper tasks --overdue  # what slipped?
```

`knapper init` writes two files: the config, and the `Templates/daily.md` the
config points at, so `knapper daily` works straight afterwards. A template you
already have is never overwritten, `--force` included — that flag is about
replacing knapper's config, not your writing.

`knapper init` is one-time; after that, knapper walks up from wherever you are
to find the nearest `knapper.config.md`, so commands work from any
subdirectory.

## One filter over every note

`orphans`, `hubs` and `broken-links` are the same question with the filter
fixed. `query` is the general form, so the combinations nobody wrote a command
for are available:

```bash
knapper query --where inlinks=0            # what orphans does
knapper query --sort inlinks:desc --limit 10   # what hubs does
knapper query --where broken>0 --field broken  # which notes broken-links names
```

The point is what those presets could not express — filtering on a note's own
fields and ranking by a computed one, in the same question:

```
$ knapper query --from Questions --where status=open --field inlinks --sort inlinks:desc
path                          inlinks
----------------------------  -------
Questions/where-to-live.md    4
Questions/which-bank.md       1
```

Frontmatter keys, Dataview inline fields and computed fields share one
namespace, so `status=open`, `cost>40` and `inlinks=0` all work the same way.
Operators are `=` `!=` `>` `<` `>=` `<=` `~` (contains), a bare name for
"has this field" and `!name` for "does not". `knapper fields` lists what a
given vault offers.

## Hard links and soft tags

A vault that has been written in for years is full of `[[COO採用]]` — square
brackets used as a highlighter, never as a promise that a note by that name
exists. knapper is right to call those broken links, and the fix is not to
soften the report. It is to write what was meant.

So knapper distinguishes two kinds of reference:

| | means | missing target | in the graph |
|---|---|---|---|
| `[[X]]` | a **hard note reference** | a broken link | an edge |
| `#X` | a **soft topic reference** | fine, that is the point | nothing |

A tag is not a note and never becomes one. It is not an orphan, not a hub, and
`broken-links` has nothing to say about it. But it is still navigable, because
a label you cannot follow is only half a label:

```
$ knapper backlinks '#COO採用'

Daily/2026-07-01.md (line 10) #COO採用

Daily/2026-07-01.md (line 22) #COO採用

Notes/Hiring.md (line 3) #COO採用/面接
```

Quote it — an unquoted `#` starts a comment in most shells. A leading `#` is
the **only** thing that selects a tag, so nothing changes for ordinary
arguments, and no tag becomes a graph node by accident. It is also
unconditional: a note whose filename really does start with `#` is reached by
writing a path for it, `./#notes.md`.

`context` takes one too, and answers with the whole subject rather than with a
note that does not exist:

```bash
knapper context '#COO採用' --format json
```

```json
{
  "kind": "tag",
  "tag": "COO採用",
  "selector": "#COO採用",
  "notes": ["Daily/2026-07-01.md", "Notes/Hiring.md", "Notes/Log.org"],
  "nested_tags": ["COO採用/面接"],
  "occurrences": [
    {
      "source": "Daily/2026-07-01.md",
      "line": 10,
      "tag": "COO採用",
      "where": "inline",
      "text": "- 求人票を書いた #COO採用"
    }
  ],
  "tasks": [{"file": "Daily/2026-07-01.md", "line": 22, "text": "面談を設定する #COO採用", "done": false}],
  "stats": {"notes": 3, "occurrences": 4}
}
```

There is no `path` and no `content`, because there is no note. `kind` is there
so a caller never has to guess which shape it got, and note `context` is
unchanged. Of `context`'s flags, `--no-content` drops each occurrence's line
text, `--max-content` truncates it and `--no-tasks` drops the tasks;
`--no-backlinks` has nothing to skip, since the occurrences *are* the
backlinks.

The matching rules are deliberately narrow, and the same in both commands:

- A tag covers the tags **nested** under it — `#work` finds `#work/hiring`,
  which is what writing a nested tag means. Every result names the tag it
  actually matched, so an exact hit is distinguishable from a nested one.
- Matching is otherwise **exact and case-sensitive**, with no Unicode
  normalisation. That is the rule `tags --find` already follows, so one vault
  cannot have two answers to "which notes carry this tag".
- An occurrence is either **inline** (`#tag` in the prose) or **declared**
  (YAML `tags:`, org `#+filetags:` and heading tags) — the same topic in the
  syntax each flavor uses for it, and `where` says which.
- Code fences, inline spans and `%%comments%%` hold no occurrences, exactly as
  they hold no links.
- Results are ordered by path, then line, so the JSON is stable between runs.
- `-A`/`-B` work as they do for note backlinks.

### Demoting a wikilink to a tag

`demote` is the migration: it rewrites the **exact** form and nothing else.

```
$ knapper demote "COO採用" --dry-run
[DRY RUN] Demoting [[COO採用]] -> #COO採用
  Would update 2 links in 2 files:
    Daily/2026-07-01.md
      9: [[COO採用]] -> #COO採用
    Daily/2026-07-02.md
      7: [[COO採用]] -> #COO採用
  ⚠️ Daily/2026-07-01.md:15: [[COO採用|採用の件]] cannot be demoted (alias)
  ⚠️ Daily/2026-07-01.md:19: [[Archives/COO採用]] cannot be demoted (path-qualified)
  ⚠️ Daily/2026-07-01.md:20: [[COO採用]] cannot be demoted (adjacent text)

Nothing was written.
```

Everything a tag cannot hold is reported rather than mangled — and rather than
passed over in silence, because those are the ones still to deal with by hand:

| reason | example |
|---|---|
| `alias` | `[[X\|the topic]]` — a tag has nowhere to keep display text |
| `anchor` | `[[X#Heading]]`, `[[X^b12]]` — a tag has no inside |
| `embed` | `![[X]]` — an embed transcludes, a tag does not |
| `path-qualified` | `[[Folder/X]]` — a path names a file, not a topic |
| `adjacent text` | `見た[[X]]の` — `#Xの` would be a *different* tag |
| `frontmatter` | a YAML value is not prose, and `#X` in one is a string |
| `org-mode` | knapper reads org and does not rewrite it, here as elsewhere |

Code fences, inline code and `%%comments%%` are neither rewritten nor
reported: there are no references in them to report. Excluded subtrees are
never touched. Only the matched span is replaced, so indentation, the rest of
the line, the trailing newline and CRLF endings all survive as they were.

`--dry-run` is available and writes nothing, exactly as for `rename` and
`move`; `--format json` gives the whole plan:

```json
{
  "kind": "demote",
  "target": "COO採用",
  "tag": "COO採用",
  "dry_run": true,
  "applied": false,
  "files_updated": ["Daily/2026-07-01.md", "Daily/2026-07-02.md"],
  "links_updated": 2,
  "edits": [{"file": "Daily/2026-07-01.md", "links": 1,
             "changes": [{"line": 9, "before": "[[COO採用]]", "after": "#COO採用"}]}],
  "skipped": [{"file": "Daily/2026-07-01.md", "line": 15,
               "text": "[[COO採用|採用の件]]", "reason": "alias"}]
}
```

`edits` and `skipped` are present in both modes and ordered by file then line.
Three things it refuses outright, before writing anything:

- A target that **names a note that exists**. That is not a relabelling, it is
  a deletion: the reference stops pointing anywhere and the note loses a
  backlink. `--allow-existing-note` does it anyway.
- A target that **cannot be spelled as a tag** — `Daily Tasks` has a space,
  `2026` has no letter. Pass `--tag work/daily` to name the tag to write.
- A **path-qualified** target. `demote` names a bare target; to write a nested
  tag, name the bare target and pass `--tag a/b`.

A target nothing links to is an error rather than a silent zero, because at
that point it is a typo far more often than it is a no-op.

Promotion the other way — `#X` back to `[[X]]` — is **intentionally not
implemented**. It is not the symmetric operation it looks like: demotion
strictly weakens a reference and needs nothing to exist, while promotion has
to invent a note, decide where it lives, and turn a label into a promise that
the vault then has to keep. A nested `#a/b` has no single note name at all,
and a tag on fifty daily notes would produce fifty new hard links to one new
file. Create the note and use `rename`, which already does that safely.

## Renames that don't break anything

The command that earns the install. Rename or move a note, and every inbound
link is rewritten — wikilinks and inline links alike:

```
$ knapper rename "Thesis" "Dissertation"
Renaming: Projects/Thesis.md -> Projects/Dissertation.md
Found 2 files with links to update
  Updated 1 links in Notes/Experiment Plan.md
  Updated 1 links in Notes/Lit Review.md

Done! Renamed and updated 2 links in 2 files.
```

```diff
 # Notes/Lit Review.md
-Related: [[Projects/Thesis]].
+Related: [[Projects/Dissertation]].

 # Notes/Experiment Plan.md
-Links back to [the thesis](../Projects/Thesis.md).
+Links back to [the thesis](../Projects/Dissertation.md).
```

Link text, folder prefixes, `#anchors`, and `%20` percent-encoding are all
preserved; external URLs and image embeds are left alone. Preview any refactor
with `--dry-run`:

```bash
knapper rename "old-name" "new-name" --dry-run
knapper move "note.md" "Archive/" --dry-run
```

### Moving a whole directory

`move` takes a directory as well as a note. The subtree moves as it stands —
notes, attachments, sidecar files, dotfiles, nested folders, byte for byte —
and every link that *resolves* into it is rewritten:

```
$ knapper move "Projects/Thesis" "Archive/" --dry-run
[DRY RUN] Moving directory: Projects/Thesis -> Archive/Thesis
  9 files (4 notes) move with it
  Would update 6 links in 3 files:
    Notes/Lit Review.md
      12: [[Projects/Thesis/README]] -> [[Archive/Thesis/README]]
      14: ![plan](Projects/Thesis/assets/plan.png) -> ![plan](Archive/Thesis/assets/plan.png)
    ...

Nothing was written.
```

The source is matched exactly — a vault-relative path, with or without a
trailing slash — and the result is always `DEST/<the directory's own name>`.
It does not rename the directory, and it does not merge into an existing one.

"Every inbound link" means every link that *points at a file in the subtree*,
not every line that mentions the path. So a bare `[[README]]` written next to
a different README is left exactly as it is, while one that meant the moved
README is qualified so it still lands there. Links inside the subtree are
untouched when the structure keeps them working, and a relative link out of
the subtree is recomputed for its new depth. Image and attachment links follow
the files they point at.

The whole plan is built before anything is written — so `--dry-run` shows the
real one, and a write that fails part-way restores what it touched:

```bash
knapper move "Projects/Thesis" "Archive/" --format json
```

```json
{
  "kind": "directory",
  "old_path": "Projects/Thesis",
  "new_path": "Archive/Thesis",
  "applied": true,
  "entries": 9,
  "notes": 4,
  "files_updated": ["Notes/Lit Review.md"],
  "links_updated": 6,
  "unsupported_links": [],
  "warnings": []
}
```

`--dry-run` adds `moves` (every file's old and new path) and `edits` (every
link change, with its line, before and after), which is enough to act on
without reading the vault again.

Both ends have to be inside the real vault tree. A source or destination that
reaches its directory through a **symlink** is refused, even when the path
looks perfectly vault-relative: `Archive/` could be a link to anywhere, and
following it would move the directory somewhere knapper can no longer see
while rewriting every inbound link to a path that no longer exists.

That is about the directories being moved. A symlinked *note* is a different
thing and is followed, as it is everywhere else in knapper: a link rewritten
inside one changes the file the link points at, and leaves the link itself a
link.

Rewritten inline links are percent-encoded for the path they now have, not
for the one they had. A directory called `Guide (v2)` produces
`[a](Archive/Guide%20%28v2%29/Note.md)`, because an unencoded `)` would end
the link at the wrong place — and the same goes for `%`, `"`, `<`, `>` and a
`#`, which would otherwise be read as the anchor. Wikilinks need none of this
and get none of it; a `<...>` destination keeps its angle brackets.

One gap it will not paper over: knapper reads org-mode but does not rewrite
it. If an inbound `.org` link points into the directory, the move stops,
reports the links under `unsupported_links`, and changes nothing —
`--allow-broken-org-links` does it anyway and still reports them.

## Repairing links that broke somewhere else

`rename` and `move` keep links intact through a refactor knapper performs.
`repair-links` is the other half: the links that broke while knapper was not
looking — a folder reorganised in Finder, an exporter that wrote a path which
has since moved, an encoding that survived one round trip too many.

```
$ knapper repair-links --dry-run
[DRY RUN] repair-links: 12 broken link occurrence(s) in 2 file(s)
  safe 6, ambiguous 2, unresolved 4

safe (6)
  Index.md:10:3  [[legacy/notes/Foo.md]] -> [[notes/Foo.md]]
      notes/Foo.md (unique-path-suffix: the only file whose path ends with "notes/Foo.md", and "notes/Foo.md" resolves back to it)
  Index.md:13:3  [[Docs/My%20Note]] -> [[Docs/My Note]]
      Docs/My Note.md (percent-decoding: decoding the target gives "Docs/My Note", and "Docs/My Note" resolves back to it)

ambiguous (2)
  Index.md:18:3  [[stale/docs/README.md]]
      missing-path: 2 files end with "docs/README.md"
      - Handbook/docs/README.md
      - Manual/docs/README.md

unresolved (4)
  Index.md:22:3  [[Roam]]
      missing-note: no file matches this target

Nothing was written. repair-links plans repairs; it never edits the vault.
```

**V1 plans and nothing else.** `--dry-run` is required — omitting it is
refused with exit 2 before the vault is even read, so a forgotten flag can
never be mistaken for asking to write. There is no `--apply` yet.

### What counts as evidence

A repair is proposed only where the filesystem settles it. Every occurrence
lands in one of three buckets:

| | means |
|---|---|
| `safe` | exactly one destination, reached by an exact structural transformation, and the link knapper would write resolves back to it |
| `ambiguous` | something matches, but choosing between the matches is a judgement about meaning |
| `unresolved` | nothing on disk could settle it |

Two bases are safe, and they are both about structure rather than about
resemblance:

- **`unique-path-suffix`** — the tail of the path is still exactly right and
  only the directories in front of it are stale. `[[legacy/notes/Foo.md]]`
  when `notes/Foo.md` is the *only* file whose path ends `notes/Foo.md`.
  Leading `../` is treated as part of the stale prefix, not followed.
- **`percent-decoding`** — decoding the target yields the link that was meant,
  and the decoded text **names a path** that exists. `[[Docs/My%20Note]]` for
  `Docs/My Note.md`.

Decoding is not a doorway to anything looser. knapper resolves a bare name by
stem and by alias when it *reads* a vault, and neither is evidence for an
edit — so a decoded target is only safe when it names its destination as a
path. `[[My%20Note]]` decodes onto the very same `Docs/My Note.md`, but gets
there by filename, and is only a suggestion.

Everything else is a suggestion at most:

- **`basename`** — one component matched and the directory did not.
  `[[old/Bar/Foo.md]]` beside `notes/Foo.md` is a resemblance, not a path, and
  so is `[[My%20Note]]` beside `Docs/My Note.md`.
- **An alias.** A name the author gave a note is not a location, so a target
  that reaches a note only through its `aliases:` stays `unresolved`.
- **`path-suffix` with more than one hit** — two files end the same way, so
  nothing is proposed and both are listed.
- **A renamed concept.** `[[Roam]]` next to a note called `RoamResearch` is a
  rename somebody performed in their head. No string distance turns that into
  evidence, and knapper does not try; it stays `unresolved`.
- **Missing dates and citation labels.** `[[2026-07-04]]` and `[[12]]` name no
  note at all, so there is nothing to repair and nothing is invented.
- **org links.** knapper reads org and does not rewrite it, here as in `move`
  and `demote`, so an org occurrence is reported with its line and never with
  a plan.

One basis is deliberately absent: replacing a *wrong* extension. Note
extensions need no repair — the resolver answers `[[notes/Foo.md]]` and
`[[notes/Foo]]` alike — so all such a rule could add is dropping an extension
that is not a note's, and `old/Foo.txt` becoming `notes/Foo.md` is a claim
about what the author meant by `.txt`.

Two rules keep it honest in the other direction. A link that already
**resolves is never a repair** — including a valid note-relative link, which
is not rewritten to vault-root form for style. And **excluded notes and
non-note leaves are valid destinations**, exactly as they are for the
resolver: `Archive/Old Plan.md` and `assets/paper.pdf` are real files a link
may point at.

Every safe proposal is checked by resolving the text knapper would write — as
a path, again, so a rewrite whose correctness depended on no other note ever
taking that name is not proposed. Nothing is published on the strength of a
search a reader would not reproduce.
Destinations always come from the vault walk, which does not descend symlinked
directories, so nothing behind one can become a repair target.

### The plan, as JSON

```bash
knapper repair-links --dry-run --format json
```

```json
{
  "kind": "repair-links",
  "dry_run": true,
  "applied": false,
  "summary": {"files": 2, "occurrences": 12, "safe": 6, "ambiguous": 2, "unresolved": 4},
  "occurrences": [
    {
      "source": "Index.md",
      "line": 10,
      "column": 3,
      "syntax": "wikilink",
      "raw": "[[legacy/notes/Foo.md]]",
      "raw_target": "legacy/notes/Foo.md",
      "target": "legacy/notes/Foo.md",
      "reason": "missing-path",
      "status": "safe",
      "candidates": [
        {"path": "notes/Foo.md", "basis": "unique-path-suffix", "matched": "notes/Foo.md"}
      ],
      "note": null,
      "edit": {
        "byte_start": 124,
        "byte_end": 147,
        "before": "[[legacy/notes/Foo.md]]",
        "after": "[[notes/Foo.md]]",
        "target_before": "legacy/notes/Foo.md",
        "target_after": "notes/Foo.md",
        "resolves_to": "notes/Foo.md"
      }
    }
  ]
}
```

- `status` is `safe`, `ambiguous` or `unresolved`; `reason` is `missing-note`,
  `missing-path`, `missing-date`, `numeric-label` or `org-link`.
- `column` is 1-based and counted in **characters**, so a line of Japanese
  reports the column a reader would count. It is `null` for org, whose masking
  does not preserve byte offsets — the line is exact, the column is not
  available, and no org occurrence carries an edit.
- `edit` carries the byte span plus the text before and after, which is
  everything an apply needs without scanning the vault again. It is `null`
  wherever `status` is not `safe`.
- Occurrences are ordered by source, then line, then column, so two runs over
  an unchanged vault produce identical bytes.

`broken-links --format json` is the same records without the plan — one
object per occurrence, with the position, the text as written and the
candidates. The two commands read one scan, so they cannot disagree about what
is broken, and `lint --check broken-links` and `query --where broken>0` still
count the same links.

One consequence worth knowing: knapper resolves a path-qualified link by
basename when the path itself misses, so `[[legacy/notes/Foo]]` to a live note
already resolves and never appears here. What does appear is the case that
basename fallback cannot rescue — a path written with its extension, or one
whose leaf is an excluded note or a non-note file.

## Built for agents — the evidence

"Agent-friendly" is cheap to say. Concretely: every query command speaks
`--format json`, the list-producing ones add `--format paths`, and the output
is made to be piped:

```bash
knapper backlinks "Projects/thesis.md" --format json | jq -r '.[].source'
knapper broken-links --format paths
knapper frontmatter set "Projects/x.md" status done
```

And `context` aggregates what a model would otherwise need five calls to
gather — content, outgoing links, backlinks, tags, headings, stats:

```bash
knapper context "Notes/Lit Review.md"
```

```json
{
  "path": "Notes/Lit Review.md",
  "title": "Lit Review",
  "content": "# Lit Review\n\nRelated: ...",
  "links": ["Missing Note", "Projects/Dissertation"],
  "backlinks": ["Projects/Dissertation.md"],
  "tags": ["parent/child", "research"],
  "headings": [{"level": 1, "text": "Lit Review", "line": 1}],
  "stats": {"chars": 95, "words": 10, "lines": 5}
}
```

To restore only the neighbourhood of a search hit, pass its physical
1-based line. Focused context defaults to three lines before and after the
hit, and includes a compact first-layer document map plus the complete heading
breadcrumb:

```bash
knapper context "Notes/Lit Review.md" --line 9
knapper context "Notes/Lit Review.md" --line 9 -B 1 -A 8 --format json
knapper context "Notes/Lit Review.md" --line 9 --section
```

`--section` selects the smallest enclosing Markdown section and cannot be
combined with explicit `-A/--after` or `-B/--before`. `--outline-depth N`
controls the map (`1` by default; `0` hides it). The map shows at most twelve
entries; larger outlines keep the first, current-neighbour and last entries
and report `total`, `omitted` and `truncated`. An H1 is the document title and
its direct children are the first layer; a single H1 without children is only
the title. When there are multiple H1s they are peer first-layer roots with no
document title; without an H1, the shallowest heading level is the first layer.
Focused mode is Markdown-only for now and rejects org-mode
notes rather than presenting a misleading outline.

Focused JSON is deterministic and has this shape:

```json
{
  "kind": "note",
  "path": "Notes/Lit Review.md",
  "document": {
    "title": "Lit Review",
    "outline_depth": 1,
    "entries": [{"index": 0, "title": "Method", "depth": 1, "start_line": 9, "end_line": 11, "current": true}],
    "total": 1, "omitted": 0, "truncated": false, "current_branch_index": 0
  },
  "focus": {
    "line": 9,
    "heading_path": ["Lit Review", "Method"],
    "enclosing_section": {"start_line": 9, "end_line": 11}
  },
  "excerpt": {"start_line": 6, "end_line": 11, "truncated": false, "content": "..."}
}
```

`--no-content` retains all focus, section and outline metadata while omitting
`excerpt.content`. `--max-content` truncates around the focused line in this
mode, rather than returning the beginning of the file; `excerpt.content` stays
source-only and `truncated: true` causes text output to print an unnumbered
truncation marker.

### The skill ships inside the binary

An agent host does not need this repository to learn how to drive knapper:

```bash
knapper skill              # print it
knapper skill --install    # write it into ~/.claude and ~/.codex
```

`install.sh` registers it for you, and `knapper self-update` re-registers it,
so the instructions an agent reads cannot drift behind the binary they
describe. The source is [`assets/knapper-skill.md`](assets/knapper-skill.md).

## Tasks

Tasks are plain markdown checkboxes. The emoji date convention popularised by
the [Obsidian Tasks](https://publish.obsidian.md/tasks/) plugin (`📅` due,
`⏳` scheduled, `✅` done, `🔁` recurring) is read and written but never
required — a vault of bare `- [ ]` items works fine.

```bash
knapper tasks --overdue                     # past due
knapper tasks --due-from 2026-08-01 --due-to 2026-08-31
knapper tasks --exclude Archive/ --tag work
knapper tasks new "write the README" --due 2026-08-01
knapper tasks done "write the README"       # exact match; --partial for substring
knapper tasks --prose-only                  # skip checkboxes in fenced examples
```

Every checkbox counts by default, including one written inside a code fence or
a `%%comment%%` — the one place knapper does not mask non-prose, and
deliberately: masking can only ever *hide* a task, and an unclosed fence blanks
the rest of a file. A visible example costs less than a missing entry on a list
you act on. `--prose-only` is the opt-out for a vault that documents its own
conventions.

Statuses are configurable: `open` `[ ]`, `wip` `[/]`, `done` `[x]`, and
`cancel` `[-]` are built in, and `knapper.config.md` can override their
markers or add custom statuses, which `knapper tasks set STATUS TEXT` and
`--status` filters then understand.

## Daily notes

```bash
knapper daily              # today (created from template if missing)
knapper daily yesterday
knapper daily 2026-01-15
```

Templates expand on creation — both Obsidian Core Templates (`{{date}}`,
`{{time}}`, `{{title}}`) and Templater date expressions:

```markdown
<!-- template -->
<< [[Daily/<% tp.date.now("YYYY-MM-DD", -1) %>]] | [[Daily/<% tp.date.now("YYYY-MM-DD", 1) %>]] >>

<!-- created file, 2026-07-28 -->
<< [[Daily/2026-07-27]] | [[Daily/2026-07-29]] >>
```

If `daily_notes.template` names a file knapper cannot read, `knapper daily`
fails and writes nothing — no note, not even the folder that would have held
it. A vault that names no template at all gets a plain `# YYYY-MM-DD`, because
that is all it asked for.

## External references

Some things belong in your notes by name but not by value: an address, a
licence key, a token. Write them as an ordinary markdown link with a
`knapper://` destination, and the value stays wherever you already keep it:

```markdown
住所: [日本橋小舟町の住所](knapper://personal/address.nihonbashi_kobunacho)
```

A reference is `knapper://<provider>/<locator>`. The **provider** is a name
*you* chose — `personal`, `work`, `family` — and the **locator** is opaque to
knapper: it is never decoded or normalised, and reaches the provider command
as written. Both halves still have a deliberate grammar. Provider names are
lowercase letters, digits, `_` and `-`; locators use ASCII letters, digits,
`.`, `_`, `-` and `/`, are at most 128 characters, and have no empty, `.` or
`..` segments. A destination outside that grammar is not a reference: `refs`
will not list it and `resolve` will refuse it. Keep names romanised —
`address.nihonbashi_kobunacho`, not `住所`.

### Finding references

```bash
knapper refs                          # every reference in the vault
knapper refs "Projects/application.md"
knapper refs --provider personal --format json
knapper refs --format paths           # just the files, ready for a pipe
```

Each result carries the vault-relative path, line, column, URI, provider,
locator and link label. References inside fenced code, inline code and
`%%comments%%` are ignored, as links there always are, and `knapper context
FILE --format json` reports the same under `references`.

A `knapper://` link is *external*: it never becomes an edge in the link
graph, never shows up as a broken link, and `rename` and `move` leave it
alone — exactly like an `https://` link.

### Configuring a provider

What actually runs lives in `$XDG_CONFIG_HOME/knapper/providers.yaml`
(`~/.config/knapper/providers.yaml` if that is unset) — **never** in
`knapper.config.md`. A vault is synced, shared and cloned; a file that decides
what gets executed must not travel with it. knapper reads no provider
configuration from a vault, and a `providers:` block in a vault config is
ignored.

```bash
knapper provider set personal -- op read 'op://Knapper/{locator}/value'
knapper provider set family -- pass show 'knapper/{locator}'
knapper provider list
knapper provider remove personal
```

```yaml
providers:
  personal:
    command: [op, read, "op://Knapper/{locator}/value"]
```

`command` is **argv**, not a shell line: knapper execs it directly, so there
is no quoting, no word splitting and no shell to escape from. Every
`{locator}` in it — in any argument — is replaced with the reference's
locator, and nothing else is substituted.

knapper is provider-agnostic. `op` above is one user's choice; `pass`,
`gopass`, `security`, `bw`, `gcloud secrets`, `vault read`, or a script of
your own work the same way, because knapper knows nothing about any of them.

### Resolving

```bash
knapper resolve "knapper://personal/address.nihonbashi_kobunacho"
knapper resolve "knapper://work/tokens/ci.deploy" --dry-run    # print the argv, run nothing
knapper resolve "knapper://work/tokens/ci.deploy" --timeout 30
```

The provider's stdout is the value. Exactly one trailing newline comes off;
after that, the remaining text, including multiple lines, is passed through
unchanged. A value must be UTF-8 text of at most 1 MiB — a reference names a
credential or an address, not a payload; anything larger or binary is refused
with exit 4.
stdin and stderr stay attached to your terminal, so a provider can still
prompt for a PIN, a passphrase or a hardware key.

knapper adds no newline of its own. This keeps redirection byte-exact; when
run directly in a terminal, the next prompt may therefore follow the value.

There is no default timeout — a provider may legitimately wait for you —
`--timeout SECS` bounds the whole resolve, including the wait for provider
stdout to close, and kills the provider process if it is still running.
Provider commands remain responsible for any further processes they start.

What knapper does *not* do: it never caches a resolved value, never writes one
anywhere, never scans one for further references, and never puts one in a
child process's arguments or environment. Failures are distinguishable by exit
status: **2** for a malformed reference or bad usage, **3** for a provider that
is not configured, **4** for a provider command that would not run, failed,
timed out, or returned nothing usable.

## Command reference

| Command | What it does |
|---------|--------------|
| `knapper init` | Write `knapper.config.md`, and the daily template it names |
| `knapper query` | Filter notes by frontmatter, inline fields and link counts |
| `knapper fields` | List what `query` can filter on |
| `knapper lint` | Vault health: `broken-links`, `orphans`, `duplicates`, `empty`, `frontmatter` |
| `knapper backlinks FILE` | Incoming links to a file, or the notes carrying a `'#tag'` |
| `knapper links FILE` | Outgoing links from a file |
| `knapper orphans` | Notes no other note links to |
| `knapper hubs` | Most-linked-to notes |
| `knapper broken-links` | Links to non-existent notes, one record per occurrence |
| `knapper repair-links --dry-run` | Plan repairs for broken links; never writes |
| `knapper rename OLD NEW` | Rename a note and update all links |
| `knapper move SRC DEST` | Move a note or a directory and update all links |
| `knapper demote TARGET` | Rewrite the exact `[[TARGET]]` into `#TARGET` |
| `knapper context FILE` | Aggregated context; add `--line N` for focused Markdown context |
| `knapper tasks` | Find and filter `- [ ]` tasks |
| `knapper tasks new / done / wip / cancel / set` | Create a task or change its status |
| `knapper daily [DATE]` | Create or get a daily note |
| `knapper frontmatter get / set / delete` | Read and write YAML frontmatter |
| `knapper tags` | List tags, or find files by tag with `--find` |
| `knapper refs [FILE]` | Find `knapper://` references, optionally for one provider |
| `knapper resolve REF` | Read one reference's value through its provider's command |
| `knapper provider list / set / remove` | Configure those commands, outside the vault |
| `knapper skill` | Print the embedded agent skill, or `--install` it |
| `knapper self-update` | Replace this binary with the newest release |

Query commands take `-f/--format`; the rest of the detail lives behind
`knapper COMMAND --help`.

## Configuration

`knapper init` writes `knapper.config.md` — a markdown file whose YAML
frontmatter holds the config, so it reads like any other note. The defaults
assume a conventional layout:

```yaml
---
vault_path: .
template_engine: templater   # or "core"
exclude:
  - Templates/               # unexpanded templates are not notes
daily_notes:
  folder: Daily
  template: Templates/daily.md
  format: YYYY-MM-DD
tasks:
  default_file: daily        # where `tasks new` writes: daily | inbox | a path
---
```

The generated file documents the rest, including custom task statuses.
Nothing in it is Obsidian-specific.

Every setting is checked when it is read. An unknown key, a key of the wrong
type, or a `template_engine` or `flavor` knapper does not implement is an
error naming the key, rather than a default quietly taking over — a config
typo otherwise changes what every command does and says nothing. This strictness
is the config's alone: an ordinary note with a broken YAML header is still read
without its frontmatter rather than aborting a scan.

No folder name is special. `Templates/` is in the generated `exclude` because
the daily-note template lives there and an unexpanded template is not one of
your notes; put templates elsewhere and the `exclude` entry moves with them.
`daily_notes.template` is read by path, so excluding its folder does not stop
`knapper daily`. If that template is configured and cannot be read, `knapper
daily` fails and writes nothing at all, rather than inventing a note body —
which is why `knapper init` creates the template it configures rather than
leaving a fresh vault one command short of working.

### Links that are meant to stay unresolved

A vault that came from somewhere else carries links that will never resolve
and are not mistakes — a tag page from an import, a name a generator emitted.
Thousands of them in a lint report bury the broken links that are real, so the
vault names them once:

```yaml
ignore_links:
  - Daily Tasks
  - Habits
  - Archive/Old Index
```

They then go unreported by `knapper lint`, `knapper broken-links`,
`knapper repair-links` and `query --where broken>0` alike — all four resolve
targets the same way and read the same ignore list, so a link is ignored by
all of them or by none.

The matching is deliberately narrow, because an ignore that reached too far
would hide a real mistake:

- An entry matches a **whole link target**, never a substring. `Daily Tasks`
  says nothing about `[[Daily Tasks Archive]]` or `[[Sub/Daily Tasks]]`.
- Comparison is **case-insensitive**, matching how knapper resolves links.
- The target compared is the one knapper resolves, so `[[Daily Tasks]]`,
  `[[Daily Tasks#2026]]`, `[[Daily Tasks|やること]]` and
  `[Daily Tasks](Daily%20Tasks.md)` are all the same entry.
- An entry may be written the way the link is written in a note:
  `[[Habits]]`, `Habits.md` and `Habits` are equivalent.
- A **path-qualified** link is its own target. To ignore `[[Archive/Old
  Index]]`, write `Archive/Old Index`.
- `exclude` and `ignore_links` answer different questions: `exclude` hides
  notes from every command, `ignore_links` only stops a link target from being
  called broken.

## Compatibility

knapper works on any folder of markdown, but each ecosystem has its own
conventions on top. **[docs/COMPATIBILITY.md](docs/COMPATIBILITY.md)**
enumerates them one by one — Obsidian core, the Tasks / Dataview / Templater
plugins, Foam, Dendron, Zettelkasten, plain markdown, Logseq, Roam, org-mode
— and marks each ✅ / ⚠️ / ❌, including the known gaps.

Everything marked ✅ there is exercised by a fixture vault under
`tests/fixtures/flavors/` and pinned by a test, so the table cannot quietly
drift away from the code.

Short version: Obsidian core, Foam, Zettelkasten and plain markdown work.
org-mode is parsed by its own reader, including `[[id:]]` and global
`[[*Heading]]` links. Dataview inline fields and Logseq properties are read,
including typed links like `[supports:: [[Note]]]`. Logseq and Roam exports
are read at the file level, ignoring block identity.

## Status

Alpha. Everything documented above is implemented and tested, but the CLI
surface may still shift before 1.0.

knapper began as a Python tool and was rewritten in Rust so that installing
it would not mean standing up an interpreter first. The two implementations
were developed side by side against
[one contract expressed as data](tests/contract/), which is how the port was
checked rather than hoped about; the Python one has since been retired, and
the cases outlived it.

## Development

```bash
git clone https://github.com/combinatrix-ai/knapper.git
cd knapper

cargo test          # unit tests, plus the contract suite against the binary
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## License

MIT

---

<p align="center">
  <i>Built for agents, by an agent.</i> 🐾
</p>
