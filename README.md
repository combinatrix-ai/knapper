# 🪨 knapper

**A CLI for personal knowledge management and LLM wikis. No bulky app. Just the CLI.**

Your knowledge base is a folder of notes. knapper lets you and your AI agents
query, maintain and reorganize it directly from the terminal — on your laptop,
over SSH, or in CI. One binary, no desktop app, daemon, server or index required.

It understands `[[wikilinks]]`, `[markdown](links.md)`, YAML frontmatter,
tags and tasks, and reads `.org` files too. Use it with an existing vault or
any linked collection of notes.

```bash
knapper backlinks "Projects/thesis.md"   # what references this file?
knapper rename "thesis" "dissertation"   # rename it and update links
knapper tasks --overdue --format json    # what needs attention?
```

[CLI manual](docs/CLI.md) · [Compatibility](docs/COMPATIBILITY.md) ·
[Releases](https://github.com/combinatrix-ai/knapper/releases)

> A *knapper* is someone who strikes obsidian and flint into usable blades.
> This one works a directory of markdown into something an agent can wield.

## Why this exists

Plain text tools can find words. Maintaining a knowledge base also means
understanding how notes, topics, metadata and tasks fit together. knapper
brings those operations to the terminal, for people and agents alike:

- **Structured queries** — filter and sort notes by frontmatter, inline fields, tags and computed link counts with `query`; discover available fields with `fields`.
- **The link graph** — inspect incoming and outgoing links, find orphans and hubs, and detect broken links to notes, headings and block IDs.
- **Link-safe refactors** — `rename` and `move` update links in both syntaxes; move whole directories and preview changes with `--dry-run`.
- **Link repair planning** — `repair-links --dry-run` proposes repairs where the filesystem supplies unambiguous evidence.
- **Tags and topics** — list tags, follow nested topics with `backlinks '#tag'` and `context '#tag'`, and use `demote` to turn topic-only wikilinks into tags.
- **Tasks** — create, filter and update checkboxes across the tree, with dates, priorities, recurring markers and configurable statuses.
- **Daily notes** — create or retrieve a dated note and expand Core Templates or supported Templater date expressions.
- **Frontmatter** — read, set and delete typed YAML fields from the shell.
- **Vault health** — find broken links, orphans, duplicate names and stubs; enforce frontmatter and required-heading rules by path or metadata, across the vault or only changed files.
- **Context for agents** — get a note's content and relationships in one call, or focus on a search-hit line with its section and heading outline. JSON output and an embedded agent skill make automation straightforward.
- **External references** — keep `knapper://` references in notes and resolve values through separately configured provider commands when needed.

Tags are Unicode-aware (`#café` and `#parent/child` both work), and one
malformed YAML header never aborts a whole-vault scan. The
[CLI manual](docs/CLI.md) covers all commands, options, configuration and
behavioral limits.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/combinatrix-ai/knapper/v0.1.0/install.sh | sh
```

Installs to `~/.local/bin`, verifies archive checksums, and registers the
embedded skill with supported agent hosts it finds. Release binaries support
macOS and Linux on arm64 and x86_64. See the [installation manual](docs/CLI.md#install)
for direct downloads, provenance verification, installer options and source builds.

`knapper self-update` verifies release provenance and installs updates.
Interactive commands check for updates in the background at most once every
six hours; set `KNAPPER_NO_UPDATE_CHECK=1` to disable that check. Notes are
not sent. Configured external-reference providers may also use the network.

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

```bash
knapper query --where 'status=open' --sort inlinks:desc
knapper context "Projects/thesis.md" --line 12 --format json
knapper move Projects/thesis Archive/thesis --dry-run
```

## Works with your notes

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

## Use alongside ripgrep

Use [ripgrep](https://github.com/BurntSushi/ripgrep) for full-text search, then
knapper to inspect structure:

```bash
rg -n "some phrase" Notes
knapper context "Notes/Example.md" --line 42 --format json
```

knapper works directly on files. App services such as Sync, Publish,
workspaces and plugin execution remain outside its scope. See the
[compatibility matrix](docs/COMPATIBILITY.md) for supported syntax and limits.

## Status

Alpha. The documented CLI features are implemented and tested, but the CLI
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
