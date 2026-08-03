# 🪨 knapper

**A headless CLI for markdown knowledge bases, built for AI agents to drive.**

knapper reads and writes the note files directly. No app to launch, no daemon,
no server, no index to build, no API keys — one binary, and nothing to install
beside it. Nothing it does touches the network, with one deliberate exception:
`knapper self-update`, which only runs when you ask for it by name. It
resolves **both** link syntaxes,
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

---

## Why this exists

Your notes are markdown on disk. Your agent is in a terminal — often one with
no desktop attached: a cron job, a CI runner, a container, a box you reached
over SSH. knapper gives that agent the operations plain shell tools can't do,
because they require understanding the *structure* of a vault:

- 🔗 **The link graph** — `backlinks`, `links`, `orphans`, `hubs`, `broken-links`
- ✂️ **Link-safe refactors** — `rename` and `move` rewrite every inbound link, in both syntaxes
- ✅ **Tasks** — query and mutate `- [ ]` checkboxes across the whole tree
- 📇 **Frontmatter** — get, set, and delete YAML fields from the shell
- 🩺 **Vault health** — `lint` finds broken links, orphans, duplicate names, stubs, missing frontmatter
- 🧠 **`context FILE`** — everything an LLM needs about one note, in a single call
- 🔐 **Secret references** — locate tagged identifiers without resolving or printing secret values

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
knapper covers the structural side rg can't see, and the two compose:

```bash
# structure from knapper, text from rg
knapper orphans --format paths | xargs rg -l "TODO"
```

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
knapper query --where broken>0 --field broken  # what broken-links does
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

### The skill ships inside the binary

An agent host does not need this repository to learn how to drive knapper:

```bash
knapper skill              # print it
knapper skill --install    # write it into ~/.claude and ~/.codex
```

`install.sh` registers it for you, and `knapper self-update` re-registers it,
so the instructions an agent reads cannot drift behind the binary they
describe. The source is [`assets/knapper-skill.md`](assets/knapper-skill.md).

There is an MCP server too, in [`knapper-mcp/`](knapper-mcp/).

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
```

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

## Secret references

knapper recognizes stable, tagged references to secrets without resolving or
printing their values:

```markdown
住所: ⟦secret:personal.home_address #address #さいたま⟧
```

List references in one note or across the vault, or find references carrying
every requested tag:

```bash
knapper secrets refs "Projects/application.md"
knapper secrets refs --format json
knapper secrets find --tag address --tag さいたま
knapper secrets find --tag service --format paths
```

Each result contains the vault-relative path, line, column, stable ID and
tags. References inside fenced code, inline code and `%%comments%%` are
ignored. `knapper context FILE --format json` includes the same safe metadata
under `secret_refs`.

These commands only discover references. They have no password-manager access
and cannot reveal secret values; a broker integration can resolve the stable
ID later without changing the markdown syntax.

## Command reference

| Command | What it does |
|---------|--------------|
| `knapper init` | Write `knapper.config.md` in the current directory |
| `knapper query` | Filter notes by frontmatter, inline fields and link counts |
| `knapper fields` | List what `query` can filter on |
| `knapper lint` | Vault health: `broken-links`, `orphans`, `duplicates`, `empty`, `frontmatter` |
| `knapper backlinks FILE` | Incoming links to a file |
| `knapper links FILE` | Outgoing links from a file |
| `knapper orphans` | Notes no other note links to |
| `knapper hubs` | Most-linked-to notes |
| `knapper broken-links` | Links to non-existent notes |
| `knapper rename OLD NEW` | Rename a note and update all links |
| `knapper move SRC DEST` | Move a note and update all links |
| `knapper context FILE` | Aggregated context for one file, for LLMs |
| `knapper tasks` | Find and filter `- [ ]` tasks |
| `knapper tasks new / done / wip / cancel / set` | Create a task or change its status |
| `knapper daily [DATE]` | Create or get a daily note |
| `knapper frontmatter get / set / delete` | Read and write YAML frontmatter |
| `knapper tags` | List tags, or find files by tag with `--find` |
| `knapper secrets refs / find` | Locate tagged secret references without resolving values |
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
