# CLI manual

The complete command and configuration manual for knapper. For a feature
overview, see the [README](../README.md); for format-specific support and
limitations, see the [compatibility matrix](COMPATIBILITY.md).

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Invocation and output](#invocation-and-output)
- [Frontmatter](#frontmatter)
- [Command reference](#command-reference)
- [One filter over every note](#one-filter-over-every-note)
- [Broken links include headings and block IDs](#broken-links-include-headings-and-block-ids)
- [Hard links and soft tags](#hard-links-and-soft-tags)
- [Renames that don't break anything](#renames-that-dont-break-anything)
- [Repairing links that broke somewhere else](#repairing-links-that-broke-somewhere-else)
- [Context and agent integration](#context-and-agent-integration)
- [Tasks](#tasks)
- [Daily notes](#daily-notes)
- [External references](#external-references)
- [Configuration](#configuration)
- [Arguments and options](#arguments-and-options)

## Install

Download the binary for your platform from
[Releases](https://github.com/combinatrix-ai/knapper/releases). It is one
file, about 5 MB, with nothing to install alongside it — no interpreter, no
virtualenv, no dependency resolution. That is the point: knapper is for
machines where you would rather not stand up a runtime.

```bash
curl -fsSL https://raw.githubusercontent.com/combinatrix-ai/knapper/v0.1.0/install.sh | sh
```

That puts the binary in `~/.local/bin`, verifies it against the release's
versioned checksum manifest, and registers the agent skill with whichever of Claude Code and
Codex it finds. `--bin-dir`, `--version` and `--skill none` change all three;
`--help` lists them.

Or take the archive yourself, from
[Releases](https://github.com/combinatrix-ai/knapper/releases):

Choose the archive matching your OS and CPU. v0.1.0 supports macOS arm64 /
x86_64 and Linux arm64 / x86_64 (musl). Windows is not a release target.

Verify its GitHub Actions provenance before extracting it:

```bash
gh attestation verify knapper-v0.1.0-aarch64-apple-darwin.tar.gz --repo combinatrix-ai/knapper
```

Linux builds are musl-linked, so they run on Alpine and in a scratch
container.

Because there is no package manager in that story, knapper upgrades itself:

```bash
knapper self-update --check   # is there a newer release?
knapper self-update           # install it
```

The updater verifies the checksum manifest's Sigstore provenance against the
repository's numeric identity, release tag and release workflow, then passes
the authenticated archive digest to its embedded installer. It never downloads
a mutable installer to execute. Installation is atomic in the current binary's
directory, which must be writable. `--yes` confirms noninteractive updates;
`--force` reinstalls the latest version; `--no-skill` preserves managed skills.

Automatic checks only notify; they never replace the executable. They run only
with a terminal on stderr, outside CI, and leave stdout unchanged. Version and
check-time cache files live under `$XDG_CACHE_HOME/knapper` (default
`~/.cache/knapper`). No notes or provider values are sent. A failed check retains
the last known notice. Explicit updates and configured provider commands may
also use the network. Deleting the cache clears update-check state.

For the first installation, the public installer verifies archive checksums;
use `gh attestation verify` above for independent provenance verification.
The installed binary's updater performs provenance verification itself.

<details>
<summary>From source</summary>

```bash
cargo install --git https://github.com/combinatrix-ai/knapper knapper
```

</details>

## Quick start

```bash
cd ~/your-notes
knapper init             # writes knapper.yaml — defaults are sane
knapper lint             # how healthy is this vault?
knapper tasks --overdue  # what slipped?
```

`knapper init` writes two files: the config, and the `Templates/daily.md` the
config points at, so `knapper daily` works straight afterwards. A template you
already have is never overwritten, `--force` included — that flag is about
replacing knapper's config, not your writing.

`knapper init` is one-time; after that, knapper walks up from wherever you are
to find the nearest `knapper.yaml`, so commands work from any
subdirectory.

## Invocation and output

```text
knapper [--config PATH] [--vault PATH] COMMAND [OPTIONS]
```

`-c/--config` selects a config file. `-v/--vault` overrides the vault path.
Both options are global and may be passed with any subcommand. Without an
explicit config, knapper searches upward for `knapper.yaml`.

Commands with `-f/--format` default to text. Use `--format json` for scripts;
list-producing commands also support paths where described below. Output
formats are command-specific: there is no global `--format` option.
Quote predicates such as `'inlinks>3'` so the shell does not interpret `>`
as redirection, and quote tag arguments such as `'#research'`.

Use `knapper --help`, `knapper COMMAND --help`, or
`knapper COMMAND SUBCOMMAND --help` for the installed version's syntax.
[Arguments and options](#arguments-and-options) below includes every public
command and subcommand. The hidden background update helper is internal.

Ordinary successful commands exit 0; operational errors exit 1, and argument
parsing errors exit 2. `lint` also exits 1 when it reports any findings,
including info or warning severity. `resolve` has the additional exit codes
listed in [Resolving](#resolving).

## Frontmatter

```bash
knapper frontmatter get Notes/Example.md --format json
knapper frontmatter get Notes/Example.md status
knapper frontmatter set Notes/Example.md status active
knapper frontmatter set Notes/Example.md score 3
knapper frontmatter set Notes/Example.md tags '[research, draft]'
knapper frontmatter delete Notes/Example.md score
```

`get` reads the whole YAML mapping or one key. `set` parses its value as YAML,
so numbers, booleans and lists retain their types; quote a YAML string inside
the shell argument when a scalar must remain a string. `delete` removes one
key. These write commands have no dry-run flag.

## Command reference

| Command | What it does |
|---------|--------------|
| [`knapper init`](#knapper-init) | Write `knapper.yaml`, and the daily template it names |
| [`knapper config check`](#knapper-config) | Validate the config without scanning the vault |
| [`knapper config schema`](#knapper-config) | Print the JSON Schema for YAML-aware editors |
| [`knapper query`](#knapper-query) | Filter notes by frontmatter, inline fields and link counts |
| [`knapper fields`](#knapper-fields) | List what `query` can filter on |
| [`knapper lint`](#knapper-lint) | Vault health: `broken-links`, `orphans`, `duplicates`, `empty`, `frontmatter`, `headings` |
| [`knapper backlinks FILE`](#knapper-backlinks) | Incoming links to a file, or the notes carrying a `'#tag'` |
| [`knapper links FILE`](#knapper-links) | Outgoing links from a file |
| [`knapper orphans`](#knapper-orphans) | Notes no other note links to |
| [`knapper hubs`](#knapper-hubs) | Most-linked-to notes |
| [`knapper broken-links`](#knapper-broken-links) | Links to missing notes, headings or block IDs, one record per occurrence |
| [`knapper repair-links --dry-run`](#knapper-repair-links) | Plan repairs for broken links; never writes |
| [`knapper rename OLD NEW`](#knapper-rename) | Rename a note and update all links |
| [`knapper move SRC DEST`](#knapper-move) | Move a note or a directory and update all links |
| [`knapper demote TARGET`](#knapper-demote) | Rewrite the exact `[[TARGET]]` into `#TARGET` |
| [`knapper context FILE`](#knapper-context) | Aggregated context; add `--line N` for focused Markdown context |
| [`knapper tasks`](#knapper-tasks) | Find and filter `- [ ]` tasks |
| [`knapper tasks new / done / wip / cancel / set`](#knapper-tasks) | Create a task or change its status |
| [`knapper daily [DATE]`](#knapper-daily) | Create or get a daily note |
| [`knapper frontmatter get / set / delete`](#knapper-frontmatter) | Read and write YAML frontmatter |
| [`knapper tags`](#knapper-tags) | List tags, or find files by tag with `--find` |
| [`knapper refs [FILE]`](#knapper-refs) | Find `knapper://` references, optionally for one provider |
| [`knapper resolve REF`](#knapper-resolve) | Read one reference's value through its provider's command |
| [`knapper provider list / set / remove`](#knapper-provider) | Configure those commands, outside the vault |
| [`knapper skill`](#knapper-skill) | Print the embedded agent skill, or `--install` it |
| [`knapper self-update`](#knapper-self-update) | Replace this binary with the newest release |

Query commands take `-f/--format`; the rest of the detail lives behind
`knapper COMMAND --help`.

## One filter over every note

`orphans` and `hubs` are preset queries. `broken-links` adds occurrence
details and applies lint policies by default. `query` is the general form, so the combinations nobody wrote a command
for are available:

```bash
knapper query --where inlinks=0            # what orphans does
knapper query --sort inlinks:desc --limit 10   # what hubs does
knapper query --where 'broken>0' --field broken  # notes with broken links, without lint policies
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

## Broken links include headings and block IDs

A link is only healthy when its complete destination exists. Resolving the
note is not enough when the link names an anchor: knapper also checks that the
heading or Obsidian block ID exists in the resolved Markdown note.

| Form | What is checked |
|---|---|
| `[[#Install]]` | the `Install` heading in the current note |
| `[[Guide#Install]]` | the `Install` heading in `Guide.md` |
| `[[#^build-step]]` or `[[^build-step]]` | the `^build-step` block ID in the current note |
| `[[Guide#^build-step]]` or `[[Guide^build-step]]` | the block ID in `Guide.md` |
| `[Install](#Install)` | a local Markdown heading anchor |
| `[Install](Guide.md#Install)` | a Markdown heading anchor in another note |

Missing anchors are ordinary broken-link findings with precise reasons:

```json
{
  "source": "Notes/Start here.md",
  "line": 12,
  "column": 5,
  "syntax": "wikilink",
  "raw": "[[Guide#Install]]",
  "raw_target": "Guide#Install",
  "target": "Guide#Install",
  "reason": "missing-heading",
  "status": "unresolved",
  "candidates": [],
  "note": "the heading does not exist in Notes/Guide.md"
}
```

The reason is `missing-heading` or `missing-block`. `broken-links --format
json` reports the occurrence, `lint --check broken-links` counts it, and
`query --where 'broken>0'` includes it in the source note's `broken` count.
`repair-links` reports it but offers no edit: the filesystem cannot infer which
heading or block the author intended.

Heading comparison is case-insensitive, percent-decoded, and ignores optional
closing `#` decoration (`## Install ##`). ATX and setext headings are indexed.
Block IDs are read from the end of prose lines. Frontmatter, fenced and inline
code, and `%%Obsidian comments%%` do not declare anchors. An escaped alias
separator in a Markdown table (`[[Guide#Install\|setup]]`) is handled as an
alias separator, not as part of the heading.

An anchor-only link stays local navigation and does not create a self-edge in
the link graph. A path-qualified anchor still creates the ordinary edge to its
note; if the anchor is absent, the same occurrence is additionally broken.

## Hard links and soft tags

```bash
knapper tags                        # list tags across the vault
knapper tags Notes/Example.md       # list tags in one note
knapper tags --find research        # find notes carrying a tag
knapper backlinks '#research'       # inspect occurrences
knapper context '#research' --format json
```


A vault that has been written in for years is full of `[[COO-hiring]]` — square
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
$ knapper backlinks '#COO-hiring'

Daily/2026-07-01.md (line 10) #COO-hiring

Daily/2026-07-01.md (line 22) #COO-hiring

Notes/Hiring.md (line 3) #COO-hiring/interviews
```

Quote it — an unquoted `#` starts a comment in most shells. A leading `#` is
the **only** thing that selects a tag, so nothing changes for ordinary
arguments, and no tag becomes a graph node by accident. It is also
unconditional: a note whose filename really does start with `#` is reached by
writing a path for it, `./#notes.md`.

`context` takes one too, and answers with the whole subject rather than with a
note that does not exist:

```bash
knapper context '#COO-hiring' --format json
```

```json
{
  "kind": "tag",
  "tag": "COO-hiring",
  "selector": "#COO-hiring",
  "notes": ["Daily/2026-07-01.md", "Notes/Hiring.md", "Notes/Log.org"],
  "nested_tags": ["COO-hiring/interviews"],
  "occurrences": [
    {
      "source": "Daily/2026-07-01.md",
      "line": 10,
      "tag": "COO-hiring",
      "where": "inline",
      "text": "- Wrote the job description #COO-hiring"
    }
  ],
  "tasks": [{"file": "Daily/2026-07-01.md", "line": 22, "text": "Schedule an interview #COO-hiring", "done": false}],
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
$ knapper demote "COO-hiring" --dry-run
[DRY RUN] Demoting [[COO-hiring]] -> #COO-hiring
  Would update 2 links in 2 files:
    Daily/2026-07-01.md
      9: [[COO-hiring]] -> #COO-hiring
    Daily/2026-07-02.md
      7: [[COO-hiring]] -> #COO-hiring
  ⚠️ Daily/2026-07-01.md:15: [[COO-hiring|hiring discussion]] cannot be demoted (alias)
  ⚠️ Daily/2026-07-01.md:19: [[Archives/COO-hiring]] cannot be demoted (path-qualified)
  ⚠️ Daily/2026-07-01.md:20: [[COO-hiring]] cannot be demoted (adjacent text)

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
| `adjacent text` | `saw[[X]]again` — `#Xagain` would be a *different* tag |
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
  "target": "COO-hiring",
  "tag": "COO-hiring",
  "dry_run": true,
  "applied": false,
  "files_updated": ["Daily/2026-07-01.md", "Daily/2026-07-02.md"],
  "links_updated": 2,
  "edits": [{"file": "Daily/2026-07-01.md", "links": 1,
             "changes": [{"line": 9, "before": "[[COO-hiring]]", "after": "#COO-hiring"}]}],
  "skipped": [{"file": "Daily/2026-07-01.md", "line": 15,
               "text": "[[COO-hiring|hiring discussion]]", "reason": "alias"}]
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
  `missing-path`, `missing-date`, `missing-heading`, `missing-block`,
  `numeric-label` or `org-link`. A missing heading or block ID is reported but
  never offered a repair candidate: the filesystem cannot infer its intended
  destination.
- `column` is 1-based and counted in **characters**, so a line of Japanese
  reports the column a reader would count. It is `null` for org, whose masking
  does not preserve byte offsets — the line is exact, the column is not
  available, and no org occurrence carries an edit.
- `edit` carries the byte span plus the text before and after, which is
  everything an apply needs without scanning the vault again. It is `null`
  wherever `status` is not `safe`.
- Occurrences are ordered by source, then line, then column, so two runs over
  an unchanged vault produce identical bytes.

`broken-links --all --format json` is the same records without the plan — one
object per occurrence, with the position, the text as written and the
candidates. Without `--all`, `broken-links` applies the same policies as
ordinary lint. `query --where 'broken>0'` remains independent of lint policies.

One consequence worth knowing: knapper resolves a path-qualified link by
basename when the path itself misses, so `[[legacy/notes/Foo]]` to a live note
already resolves and never appears here. What does appear is the case that
basename fallback cannot rescue — a path written with its extension, or one
whose leaf is an excluded note or a non-note file.

## Context and agent integration

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
describe. The source is [`assets/knapper-skill.md`](../assets/knapper-skill.md).

## Tasks

Use explicit `YYYY-MM-DD` values for task date filters and creation flags.
`tasks new` uses `--file` when supplied, otherwise `tasks.default_file`.
Use `--file` to scope status changes and `--partial` only when substring
matching is intended. The full task filter and subcommand options appear
in [Arguments and options](#arguments-and-options).


Tasks are plain markdown checkboxes. The emoji date convention popularised by
the [Obsidian Tasks](https://publish.obsidian.md/tasks/) plugin (`📅` due,
`⏳` scheduled, `🛫` start, `✅` done, `🔁` recurring) is read and written but never
required — a vault of bare `- [ ]` items works fine.

```bash
knapper tasks --overdue                     # past due
knapper tasks --available                   # no start date, or start date <= today
knapper tasks --due-from 2026-08-01 --due-to 2026-08-31
knapper tasks --start-on 2026-08-01
knapper tasks --scheduled-from 2026-08-01 --scheduled-to 2026-08-07
knapper tasks --exclude Archive/ --tag work
knapper tasks new "write the README" --start 2026-07-28 --due 2026-08-01
knapper tasks done "write the README"       # exact match; --partial for substring
knapper tasks --prose-only                  # skip checkboxes in fenced examples
```

Every checkbox counts by default, including one written inside a code fence or
a `%%comment%%` — the one place knapper does not mask non-prose, and
deliberately: masking can only ever *hide* a task, and a visible example costs
less than a missing entry on a list you act on. `--prose-only` is the opt-out
for a vault that documents its own conventions.

`--available` is also opt-in: it hides only tasks whose `🛫` start date is in
the future. Tasks without a start date remain visible, and plain `knapper
tasks` keeps its established whole-list behaviour. A scheduled date says when
you intend to work; it does not make a task unavailable.

Statuses are configurable: `open` `[ ]`, `wip` `[/]`, `done` `[x]`, and
`cancel` `[-]` are built in, and `knapper.yaml` can override their
markers or add custom statuses, which `knapper tasks set STATUS TEXT` and
`--status` filters then understand.

## Daily notes

`--path-only` changes the output, not the operation: it still creates the
note if missing. JSON output reports `path`, `date`, and `created`.


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
Address: [Nihonbashi Kobunacho address](knapper://personal/address.nihonbashi_kobunacho)
```

A reference is `knapper://<provider>/<locator>`. The **provider** is a name
*you* chose — `personal`, `work`, `family` — and the **locator** is opaque to
knapper: it is never decoded or normalised, and reaches the provider command
as written. Both halves still have a deliberate grammar. Provider names are
lowercase letters, digits, `_` and `-`; locators use ASCII letters, digits,
`.`, `_`, `-` and `/`, are at most 128 characters, and have no empty, `.` or
`..` segments. A destination outside that grammar is not a reference: `refs`
will not list it and `resolve` will refuse it. Keep locators machine-readable —
`address.nihonbashi_kobunacho`, not `Street Address`.

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
`knapper.yaml`. A vault is synced, shared and cloned; a file that decides
what gets executed must not travel with it. knapper reads no provider
configuration from a vault, and a `providers:` block in a vault config is
refused.

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

## Configuration

`knapper init` writes `knapper.yaml`, a plain YAML file. It includes a
`yaml-language-server` schema modeline, so editors that support the YAML
Language Server can offer completion, hover documentation and diagnostics.
Only `knapper.yaml` is discovered; the former Markdown-frontmatter
`knapper.config.md` is not a fallback.
The defaults assume a conventional layout:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/combinatrix-ai/knapper/main/schema/knapper.schema.json
vault_path: .
template_engine: templater   # or "core"
exclude:
  - Templates/               # unexpanded templates are not notes
lint:
  rules:
    broken-links:
      enabled: true
      # include: [Projects/]  # optional path-prefix scope
      # exclude: [Projects/archive/]
    # orphans:
    #   enabled: false
    # duplicates:
    #   include: [Notes/]
  # paths:
  #   - path: ^Questions/
  #     frontmatter:
  #       required: [status]
  #       fields:
  #         status:
  #           type: string
  #           enum: [open, deciding, decided, dropped]
daily_notes:
  folder: Daily
  template: Templates/daily.md
  format: YYYY-MM-DD
tasks:
  default_file: daily        # where `tasks new` writes: daily | inbox | a path
```

The schema is also available locally with `knapper config schema`. `knapper
config check` is the authoritative runtime validator: it catches unknown keys,
wrong types, unsupported values and semantic errors such as ambiguous task
status markers without scanning any notes. Nothing in the config is
Obsidian-specific.

### Selecting files for lint

```bash
knapper lint Notes/One.md "Notes/Two words.md"
knapper lint --diff                 # staged + unstaged + untracked notes
knapper lint --diff main            # working-tree changes against this ref + untracked notes
knapper lint Notes/One.md --check headings --format json
```

File arguments are vault-relative or absolute and may omit the note extension.
Missing, excluded and out-of-vault files are rejected. File arguments and `--diff`
cannot be combined. Git selection handles spaces, Unicode, renames and an unborn
repository; deleted files, ignored untracked files and non-notes are not selected.
A missing Git repository or invalid ref is an error, never a whole-vault fallback.
An empty selection reports zero issues and exits 0. JSON includes `scope.files`
for either selection mode; plain `lint` still checks the whole configured vault.

Selection limits the source files reported, not the link-resolution universe:
links may point to unchanged files, incoming links still prevent false orphans,
and duplicate checks compare a selected note against unchanged peers in rule scope.
Local checks only read selected note bodies (plus peers when a duplicate policy
needs them). Graph checks and graph-field filters still read the full vault.
Run a full lint after deleting notes or changing rules to catch effects on
unchanged notes. `--diff` reads the working tree, not a staged-content snapshot.

### Configuring lint rules

Heading links accept visible titles and common Markdown slugs (`#quick-start`
for `Quick Start`), including numbered duplicate slugs. Required-heading policies
still compare heading titles, not slugs.

`lint.rules` controls the six checks reported by `knapper lint`:
`broken-links`, `orphans`, `duplicates`, `empty`, `frontmatter` and `headings`. Every rule
is enabled with no configuration, preserving the default all-checks report.
Set `enabled: false` to omit a rule from a plain `knapper lint`; an explicit
`knapper lint --check RULE` always runs that rule. `--check` may be repeated.

`include` and `exclude` are optional vault-relative path-prefix lists. A file
must be under at least one `include` prefix (an absent or empty list means all
files), and `exclude` wins when both match. Broken links are filtered by their
source file. The other file checks use the file being reported. Duplicate
groups are kept only when at least two paths remain in scope, and only those
paths are shown. Unknown rule names, fields and value types are configuration
errors, as are unknown `--check` names.

For finer-grained policies, `lint.paths` is an ordered list. Each entry
requires `path`, `where`, or both. `path` is a Rust regex matched against the
complete vault-relative path without adding anchors. `where` is a nonempty list
of the same expressions accepted by `query --where`: frontmatter, inline fields,
tags and computed fields are available. All expressions and the path must match. A path entry may set any check to a boolean shorthand
or a block; when several entries match, the last entry that names that check
wins, while an unnamed check inherits the global rule. A block enables its
check by default, so a path can opt into a globally disabled check. The
`broken-links` block accepts a `pattern` regex for normalized unresolved link
targets. `frontmatter` accepts `required` keys and `fields` rules with
`type` (`string`, `number`, `boolean`, `date`, `list`, `object`), scalar
`enum`, and `required_if: {field, equals}`. Field errors are reported with the
file and field and are included in `frontmatter_errors` and `total_issues`.
An empty YAML value (`key:` or `key: null`) counts as unset: optional fields
may remain empty, while `required` and a matching `required_if` still report it.

For example, apply a policy by note type rather than filename:

```yaml
lint:
  paths:
    - path: '^Logs/'
      where: ['type=work-log-manifest', '!archived']
      frontmatter:
        required: [context, status]
      headings:
        required: [Result]
```

The filter selects notes; it does not require the filter field to exist.
A separate path-only rule can require `type` so missing or mistyped types do
not silently fall outside the intended policy.

Existing in-vault directory links, such as `[docs](../docs/)`, are valid even
without a README. Directories are not note nodes or duplicate/orphan candidates.
Directory lookup never falls back to an unrelated basename, follows a directory
symlink, or resolves a path outside the vault.

Missing headings and block IDs are part of `broken-links`, so they inherit its
global and path-specific `include`, `exclude`, `enabled`, and `pattern`
settings. For an anchor failure, `pattern` sees the complete lint target after
path normalization, such as `Guide#Install` or `#^build-step`; the anchor's
written spelling is retained.

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
`knapper repair-links` and `query --where 'broken>0'` alike — all four resolve
targets the same way and read the same ignore list, so a link is ignored by
all of them or by none.

The matching is deliberately narrow, because an ignore that reached too far
would hide a real mistake:

- An entry matches a **whole link target**, never a substring. `Daily Tasks`
  says nothing about `[[Daily Tasks Archive]]` or `[[Sub/Daily Tasks]]`.
- Comparison is **case-insensitive**, matching how knapper resolves links.
- The target compared is the one knapper resolves, so `[[Daily Tasks]]`,
  `[[Daily Tasks#2026]]`, `[[Daily Tasks|things to do]]` and
  `[Daily Tasks](Daily%20Tasks.md)` are all the same entry.
- An entry may be written the way the link is written in a note:
  `[[Habits]]`, `Habits.md` and `Habits` are equivalent.
- A **path-qualified** link is its own target. To ignore `[[Archive/Old
  Index]]`, write `Archive/Old Index`.
- `exclude` and `ignore_links` answer different questions: `exclude` hides
  notes from every command, `ignore_links` only stops a link target from being
  called broken.

### Required headings

Require section titles for selected paths with `lint.paths`:

```yaml
lint:
  paths:
    - path: '^Wiki/acquaintances/[^/]+\.md$'
      headings:
        required: [Bio, ユーザーとの関係, 交流履歴, 出典]
```

Run `knapper lint --check headings --format json`. Each missing title produces
an issue with `type: headings`, `file`, `heading`, `detail`, and `severity`;
`summary.missing_headings` counts missing titles. Text output lists the file and
missing title. Like other lint checks, the command exits 1 when any findings
are reported and 0 when `summary.total_issues` is zero. Both text and JSON
output retain the complete report, including findings with info or warning severity.

Titles use the same normalization as heading anchors (case-insensitive, surrounding
whitespace and trailing ATX decoration ignored; inline markup remains literal). ATX and Setext Markdown headings and Org headings count;
frontmatter, closed code blocks, and comments do not. Level, order, uniqueness,
and section contents are not constrained. Duplicate normalized requirements
produce only one issue. An empty `required` list imposes no constraint. No
required headings are assumed for unconfigured paths.

`headings: false` disables a matching path; `true` enables it without requirements.
Global `lint.rules.headings` supports enabled/include/exclude, and later matching
path rules replace earlier heading policies in full (including global scope),
following the other lint checks. Top-level `exclude` always applies.

### Broken-link inspection scope

`knapper broken-links` respects the broken-links policies used by ordinary
`knapper lint`, including enablement, include/exclude, ordered path and metadata
rules, and target patterns. Its output retains detailed occurrence locations.
Use `knapper broken-links --all` to bypass those lint policies for investigation;
top-level `exclude` and `ignore_links` still apply. `repair-links` retains its
unfiltered diagnostic scope.

The complete accepted configuration keys, types and defaults are defined
in the [JSON Schema](../schema/knapper.schema.json). The
[starter configuration](../src/default_config.yaml) includes task status,
created-date and template settings. Validate changes with `knapper config check`.

## Arguments and options

The following help output is captured from the current source build. When
changing CLI arguments, refresh the affected entries with the matching
`--help` command. Global options are repeated so each entry stands alone.

[knapper](#knapper) · [knapper links](#knapper-links) · [knapper backlinks](#knapper-backlinks) · [knapper orphans](#knapper-orphans) · [knapper hubs](#knapper-hubs) · [knapper broken-links](#knapper-broken-links) · [knapper repair-links](#knapper-repair-links) · [knapper init](#knapper-init) · [knapper config](#knapper-config) · [knapper config check](#knapper-config-check) · [knapper config schema](#knapper-config-schema) · [knapper skill](#knapper-skill) · [knapper self-update](#knapper-self-update) · [knapper context](#knapper-context) · [knapper frontmatter](#knapper-frontmatter) · [knapper frontmatter get](#knapper-frontmatter-get) · [knapper frontmatter set](#knapper-frontmatter-set) · [knapper frontmatter delete](#knapper-frontmatter-delete) · [knapper lint](#knapper-lint) · [knapper daily](#knapper-daily) · [knapper rename](#knapper-rename) · [knapper demote](#knapper-demote) · [knapper query](#knapper-query) · [knapper fields](#knapper-fields) · [knapper move](#knapper-move) · [knapper tasks](#knapper-tasks) · [knapper tasks new](#knapper-tasks-new) · [knapper tasks done](#knapper-tasks-done) · [knapper tasks wip](#knapper-tasks-wip) · [knapper tasks cancel](#knapper-tasks-cancel) · [knapper tasks set](#knapper-tasks-set) · [knapper tags](#knapper-tags) · [knapper refs](#knapper-refs) · [knapper resolve](#knapper-resolve) · [knapper provider](#knapper-provider) · [knapper provider list](#knapper-provider-list) · [knapper provider set](#knapper-provider-set) · [knapper provider remove](#knapper-provider-remove)

### knapper

```text
knapper - a headless CLI for markdown knowledge bases.

Reads and writes markdown files directly; nothing needs to be running.
Resolves both [[wikilinks]] and [inline](links.md).

Usage: knapper [OPTIONS] <COMMAND>

Commands:
  links         List outgoing links from a file
  backlinks     List incoming links to a file, or the notes carrying a '#tag'
  orphans       Find orphan notes (not linked by any other note)
  hubs          Find hub notes (most linked-to notes)
  broken-links  Find links to missing notes, headings, or block IDs
  repair-links  Plan repairs for broken links. Reports only; writes nothing
  init          Initialize a new knapper config file
  config        Inspect or print the knapper YAML configuration schema
  skill         Print the embedded agent skill, or register it with the agents here
  self-update   Verify release provenance and replace this binary with the newest release
  context       Get comprehensive context for a file or a '#tag' (for LLMs)
  frontmatter   Read and write frontmatter
  lint          Check vault health (exit 1 when any issues are found)
  daily         Create or get daily note
  rename        Rename a note and update all links to it
  demote        Rewrite a hard [[target]] into a soft #tag
  query         Filter notes by frontmatter, inline fields, and link counts
  fields        List what query can filter on, computed and declared
  move          Move a note or a directory to a different folder and update all links
  tasks         Find and filter tasks (- [ ] items) in the vault
  tags          List tags
  refs          List external references (knapper://provider/locator) in the vault
  resolve       Read the value behind one reference, through its provider's command
  provider      Configure the commands that resolve references
  help          Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>
          Path to config file

  -v, --vault <VAULT>
          Path to vault (overrides config)

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

### knapper links

```text
List outgoing links from a file

Usage: knapper links [OPTIONS] <FILE>

Arguments:
  <FILE>  

Options:
  -A, --after <AFTER>    Lines after match [default: 0]
  -c, --config <CONFIG>  Path to config file
  -B, --before <BEFORE>  Lines before match [default: 0]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -f, --format <FORMAT>  [default: text]
  -h, --help             Print help
```

### knapper backlinks

```text
List incoming links to a file, or the notes carrying a '#tag'

Usage: knapper backlinks [OPTIONS] <FILE|#TAG>

Arguments:
  <FILE|#TAG>  A note, or a '#tag' to look up as a topic

Options:
  -A, --after <AFTER>    Lines after match [default: 0]
  -c, --config <CONFIG>  Path to config file
  -B, --before <BEFORE>  Lines before match [default: 0]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -f, --format <FORMAT>  [default: text]
  -h, --help             Print help
```

### knapper orphans

```text
Find orphan notes (not linked by any other note)

Usage: knapper orphans [OPTIONS]

Options:
  -c, --config <CONFIG>  Path to config file
      --include-special  Include hidden folders
  -f, --format <FORMAT>  [default: text]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper hubs

```text
Find hub notes (most linked-to notes)

Usage: knapper hubs [OPTIONS]

Options:
  -c, --config <CONFIG>  Path to config file
  -l, --limit <LIMIT>    [default: 10]
  -f, --format <FORMAT>  [default: text]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper broken-links

```text
Find links to missing notes, headings, or block IDs

Usage: knapper broken-links [OPTIONS]

Options:
      --all              Ignore lint enablement, scopes and patterns; retain exclude and ignore_links
  -c, --config <CONFIG>  Path to config file
  -f, --format <FORMAT>  [default: text]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper repair-links

```text
Plan repairs for links that are already broken.

This is the other half of `rename` and `move`: they keep links intact through a
refactor knapper performs, and this reports the ones that broke while knapper was
not looking -- a folder reorganised elsewhere, an exporter's stale path.

A repair is proposed only where the filesystem settles it: exactly one file, reached
by an exact structural transformation of the target as written. A resemblance is
not evidence, so a renamed concept, a missing date and a citation label are reported
and left alone.

V1 is read-only. --dry-run is required, and nothing is ever written.

Usage: knapper repair-links [OPTIONS]

Options:
  -c, --config <CONFIG>
          Path to config file

      --dry-run
          Required: this command only ever plans

  -f, --format <FORMAT>
          [default: text]

  -v, --vault <VAULT>
          Path to vault (overrides config)

  -h, --help
          Print help (see a summary with '-h')
```

### knapper init

```text
Initialize a new knapper config file

Usage: knapper init [OPTIONS]

Options:
  -c, --config <CONFIG>  Path to config file
  -f, --force            
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper config

```text
Inspect or print the knapper YAML configuration schema

Usage: knapper config [OPTIONS] <COMMAND>

Commands:
  check   Validate the discovered config without scanning the vault
  schema  Print the JSON Schema used by YAML-aware editors
  help    Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>  Path to config file
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper config check

```text
Validate the discovered config without scanning the vault

Usage: knapper config check [OPTIONS]

Options:
  -c, --config <CONFIG>  Path to config file
  -f, --format <FORMAT>  [default: text]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper config schema

```text
Print the JSON Schema used by YAML-aware editors

Usage: knapper config schema [OPTIONS]

Options:
  -c, --config <CONFIG>  Path to config file
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper skill

```text
Print the embedded agent skill, or register it with the agents here

Usage: knapper skill [OPTIONS]

Options:
  -c, --config <CONFIG>  Path to config file
      --install          Write it into every agent host found on this machine
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper self-update

```text
Verify release provenance and replace this binary with the newest release

Usage: knapper self-update [OPTIONS]

Options:
  -c, --config <CONFIG>  Path to config file
      --force            Reinstall the latest release even if the version matches
      --no-skill         Keep externally managed agent skills unchanged
  -v, --vault <VAULT>    Path to vault (overrides config)
      --check            Report whether a newer release exists, without installing it
  -y, --yes              Skip the confirmation prompt
  -h, --help             Print help
```

### knapper context

```text
Get comprehensive context for a file or a '#tag' (for LLMs)

Usage: knapper context [OPTIONS] <FILE|#TAG>

Arguments:
  <FILE|#TAG>  A note, or a '#tag' to read as a virtual topic

Options:
  -c, --config <CONFIG>            Path to config file
      --no-content                 Exclude file content
      --no-backlinks               Exclude backlinks (faster)
  -v, --vault <VAULT>              Path to vault (overrides config)
      --no-tasks                   Exclude tasks
      --max-content <MAX_CONTENT>  Truncate content to N characters
      --line <N>                   Focus on physical 1-based line N
  -B, --before <N>                 Lines before the focused line (default: 3)
  -A, --after <N>                  Lines after the focused line (default: 3)
      --section                    Show the smallest enclosing Markdown section
      --outline-depth <N>          Focused document map depth (default: 1; 0 hides it)
  -f, --format <FORMAT>            [default: text]
  -h, --help                       Print help
```

### knapper frontmatter

```text
Read and write frontmatter

Usage: knapper frontmatter [OPTIONS] <COMMAND>

Commands:
  get     Get frontmatter value(s)
  set     Set frontmatter value
  delete  Delete frontmatter key
  help    Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>  Path to config file
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper frontmatter get

```text
Get frontmatter value(s)

Usage: knapper frontmatter get [OPTIONS] <FILE> [KEY]

Arguments:
  <FILE>  
  [KEY]   

Options:
  -c, --config <CONFIG>  Path to config file
  -f, --format <FORMAT>  [default: text]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper frontmatter set

```text
Set frontmatter value

Usage: knapper frontmatter set [OPTIONS] <FILE> <KEY> <VALUE>

Arguments:
  <FILE>   
  <KEY>    
  <VALUE>  

Options:
  -c, --config <CONFIG>  Path to config file
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper frontmatter delete

```text
Delete frontmatter key

Usage: knapper frontmatter delete [OPTIONS] <FILE> <KEY>

Arguments:
  <FILE>  
  <KEY>   

Options:
  -c, --config <CONFIG>  Path to config file
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper lint

```text
Check vault health (exit 1 when any issues are found)

Usage: knapper lint [OPTIONS] [FILE]...

Arguments:
  [FILE]...  Vault-relative or absolute note filenames (repeatable)

Options:
  -c, --config <CONFIG>  Path to config file
      --check <RULE>     Run one configured lint rule (repeatable) [possible values: broken-links, orphans, duplicates, empty, frontmatter, headings]
      --diff [<REF>]     Only changed notes: staged, unstaged and untracked; optionally compare to REF
  -v, --vault <VAULT>    Path to vault (overrides config)
  -f, --format <FORMAT>  [default: text]
  -h, --help             Print help
```

### knapper daily

```text
Create or get daily note

Usage: knapper daily [OPTIONS] [DATE]

Arguments:
  [DATE]  

Options:
  -c, --config <CONFIG>  Path to config file
  -p, --path-only        
  -f, --format <FORMAT>  [default: text]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper rename

```text
Rename a note and update all links to it

Usage: knapper rename [OPTIONS] <OLD> <NEW>

Arguments:
  <OLD>  
  <NEW>  

Options:
  -c, --config <CONFIG>  Path to config file
      --dry-run          
  -f, --format <FORMAT>  [default: text]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper demote

```text
Demote a wikilink target to a tag.

[[X]] is a hard note reference: a missing target is a broken link. #X is a soft
topic reference: it labels a note and promises no note exists. This rewrites the
exact form -- [[X]] becomes #X -- and reports every occurrence it will not touch,
such as [[X|alias]], [[X#heading]], ![[X]] and [[folder/X]].

Usage: knapper demote [OPTIONS] <TARGET>

Arguments:
  <TARGET>
          A wikilink target: X or [[X]]

Options:
  -c, --config <CONFIG>
          Path to config file

      --tag <TAG>
          Write this tag instead of the target itself

      --dry-run
          

  -v, --vault <VAULT>
          Path to vault (overrides config)

      --allow-existing-note
          Demote even though the target names a note that exists

  -f, --format <FORMAT>
          [default: text]

  -h, --help
          Print help (see a summary with '-h')
```

### knapper query

```text
Filter notes by frontmatter, inline fields, and link counts

Usage: knapper query [OPTIONS]

Options:
  -c, --config <CONFIG>  Path to config file
  -w, --where <WHERE>    Filter, e.g. status=open, inlinks>3, tags~x
      --field <FIELD>    Extra column to show (repeatable)
  -v, --vault <VAULT>    Path to vault (overrides config)
      --from <FROM>      Only notes under this path
      --sort <SORT>      Sort by a field, e.g. inlinks:desc
  -l, --limit <LIMIT>    
  -f, --format <FORMAT>  [default: text]
  -h, --help             Print help
```

### knapper fields

```text
List what query can filter on, computed and declared

Usage: knapper fields [OPTIONS]

Options:
  -c, --config <CONFIG>  Path to config file
  -f, --format <FORMAT>  [default: text]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper move

```text
Move a note or a directory to a different folder and update all links

Usage: knapper move [OPTIONS] <SOURCE> <DESTINATION>

Arguments:
  <SOURCE>       
  <DESTINATION>  

Options:
  -c, --config <CONFIG>         Path to config file
      --dry-run                 
      --allow-broken-org-links  Move a directory even though inbound org links will break
  -v, --vault <VAULT>           Path to vault (overrides config)
  -f, --format <FORMAT>         [default: text]
  -h, --help                    Print help
```

### knapper tasks

```text
Find and filter tasks (- [ ] items) in the vault

Usage: knapper tasks [OPTIONS] [COMMAND]

Commands:
  new     Create a new task
  done    Mark a task as done
  wip     Mark a task as in progress
  cancel  Mark a task as cancelled
  set     Transition a task to a named status
  help    Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>
          Path to config file
  -v, --vault <VAULT>
          Path to vault (overrides config)
      --all
          Include completed tasks
      --done
          Show only completed tasks
      --recurring
          Show only recurring tasks
      --overdue
          Show only overdue tasks
      --has-date
          Show only tasks with any date marker
      --available
          Hide tasks whose start date is after today
      --due-on <DUE_ON>
          
      --due-from <DUE_FROM>
          
      --due-to <DUE_TO>
          
      --scheduled-on <SCHEDULED_ON>
          
      --scheduled-from <SCHEDULED_FROM>
          
      --scheduled-to <SCHEDULED_TO>
          
      --start-on <START_ON>
          
      --start-from <START_FROM>
          
      --start-to <START_TO>
          
      --created-from <CREATED_FROM>
          
      --created-to <CREATED_TO>
          
      --done-from <DONE_FROM>
          
      --done-to <DONE_TO>
          
      --file <FILE>
          Filter by file/folder path
      --exclude <EXCLUDE>
          Exclude files/folders (repeatable)
      --tag <TAG>
          Filter by tag
      --status <STATUS>
          Filter by status name (repeatable)
  -A, --after <AFTER>
          Lines of context after task
  -B, --before <BEFORE>
          Lines of context before task
  -C, --context <CONTEXT>
          Lines of context on both sides
  -b, --breadcrumbs
          Show heading hierarchy
      --group <GROUP>
          Group results [possible values: file, date]
      --prose-only
          Skip checkboxes in code fences, %%comments%% and inline code
  -f, --format <FORMAT>
          [default: text]
  -h, --help
          Print help
```

### knapper tasks new

```text
Create a new task

Usage: knapper tasks new [OPTIONS] <TEXT>

Arguments:
  <TEXT>  

Options:
  -c, --config <CONFIG>        Path to config file
      --file <FILE>            Target file (overrides config default)
      --due <DUE>              Due date (YYYY-MM-DD)
  -v, --vault <VAULT>          Path to vault (overrides config)
      --scheduled <SCHEDULED>  Scheduled date (YYYY-MM-DD)
      --start <START>          Start date (YYYY-MM-DD)
      --recurring <RECURRING>  Recurrence (e.g. 'every 3 months')
      --priority <PRIORITY>    [possible values: high, medium, low]
  -h, --help                   Print help
```

### knapper tasks done

```text
Mark a task as done

Usage: knapper tasks done [OPTIONS] <TEXT>

Arguments:
  <TEXT>  

Options:
  -c, --config <CONFIG>  Path to config file
      --file <FILE>      
      --partial          Use partial matching (contains)
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper tasks wip

```text
Mark a task as in progress

Usage: knapper tasks wip [OPTIONS] <TEXT>

Arguments:
  <TEXT>  

Options:
  -c, --config <CONFIG>  Path to config file
      --file <FILE>      
      --partial          
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper tasks cancel

```text
Mark a task as cancelled

Usage: knapper tasks cancel [OPTIONS] <TEXT>

Arguments:
  <TEXT>  

Options:
  -c, --config <CONFIG>  Path to config file
      --file <FILE>      
      --partial          
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper tasks set

```text
Transition a task to a named status

Usage: knapper tasks set [OPTIONS] <STATUS> <TEXT>

Arguments:
  <STATUS>  
  <TEXT>    

Options:
  -c, --config <CONFIG>  Path to config file
      --file <FILE>      
      --partial          
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper tags

```text
List tags

Usage: knapper tags [OPTIONS] [FILE]

Arguments:
  [FILE]  

Options:
  -c, --config <CONFIG>  Path to config file
      --find <FIND>      
  -f, --format <FORMAT>  [default: text]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper refs

```text
List external references (knapper://provider/locator) in the vault

Usage: knapper refs [OPTIONS] [FILE]

Arguments:
  [FILE]  

Options:
  -c, --config <CONFIG>      Path to config file
      --provider <PROVIDER>  Only references naming this provider
  -f, --format <FORMAT>      [default: text]
  -v, --vault <VAULT>        Path to vault (overrides config)
  -h, --help                 Print help
```

### knapper resolve

```text
Read the value behind one reference, through its provider's command

Usage: knapper resolve [OPTIONS] <REFERENCE>

Arguments:
  <REFERENCE>  knapper://<provider>/<locator>

Options:
  -c, --config <CONFIG>  Path to config file
      --timeout <SECS>   Give up after SECS
      --dry-run          Print the command instead of running it
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper provider

```text
Configure the commands that resolve references

Usage: knapper provider [OPTIONS] <COMMAND>

Commands:
  list    List the configured providers and the commands they run
  set     Define or replace one provider's command
  remove  Forget one provider
  help    Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>  Path to config file
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper provider list

```text
List the configured providers and the commands they run

Usage: knapper provider list [OPTIONS]

Options:
  -c, --config <CONFIG>  Path to config file
  -f, --format <FORMAT>  [default: text]
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper provider set

```text
Define or replace one provider's command

Usage: knapper provider set [OPTIONS] <NAME> -- <COMMAND>...

Arguments:
  <NAME>        
  <COMMAND>...  Command after `--`, e.g. -- op read 'op://Vault/{locator}/value'

Options:
  -c, --config <CONFIG>  Path to config file
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```

### knapper provider remove

```text
Forget one provider

Usage: knapper provider remove [OPTIONS] <NAME>

Arguments:
  <NAME>  

Options:
  -c, --config <CONFIG>  Path to config file
  -v, --vault <VAULT>    Path to vault (overrides config)
  -h, --help             Print help
```
