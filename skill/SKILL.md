---
name: knapper
description: Headless CLI for a directory of markdown notes. Use when creating daily notes, querying links and backlinks, reading/writing frontmatter, managing tasks, or renaming/moving notes without breaking links - especially where no note-taking app is running. Triggers on: daily note, backlinks, wikilinks, frontmatter, tasks, vault, knowledge base, markdown notes.
---

# knapper

CLI for AI agents to work on a directory of markdown notes -- an Obsidian
vault, a Foam or Dendron workspace, a Zettelkasten, or any other folder of
`.md` files. It edits files directly, so no app needs to be running.

If Obsidian itself is open on this machine, prefer the official `obsidian`
CLI (https://help.obsidian.md/cli); it reaches Bases, Sync, Publish and
workspaces, which knapper cannot. knapper is for headless environments.

## Setup

Ensure knapper is installed and a `knapper.config.md` exists in the vault root:

```bash
cd ~/your-vault
knapper init  # creates config if missing
```

## Commands

### Daily Notes

```bash
knapper daily                # today's note (creates if missing)
knapper daily yesterday
knapper daily 2026-01-26
knapper daily --path-only    # just print path
```

Templater expressions (`<% tp.date.now() %>`) are expanded automatically.

### Links

```bash
knapper links FILE           # outgoing [[wikilinks]]
knapper backlinks FILE       # files that link TO this file
```

### Frontmatter

```bash
knapper frontmatter get FILE
knapper frontmatter set FILE key value
knapper frontmatter delete FILE key
```

### Tags

```bash
knapper tags                 # all tags in vault
knapper tags FILE            # tags in specific file
```

### Graph Analysis

```bash
knapper orphans              # notes with no incoming links
knapper hubs --limit 10      # most-linked notes
knapper broken-links         # links to non-existent files
```

### Refactoring

```bash
knapper rename "old" "new"   # rename + update all links
knapper move "note" "folder/" # move + update all links
knapper rename X Y --dry-run # preview without changing
```

### Lint

```bash
knapper lint                 # check vault health
knapper lint --format json   # structured output
```

Checks: broken-links, orphans, duplicates, empty, frontmatter

## Config

Located at `knapper.config.md` in vault root:

```yaml
---
vault_path: .
template_engine: templater
daily_notes:
  folder: Diary
  template: Templates/Daily.md
  format: YYYY-MM-DD
---
```

## Output Formats

All commands support `--format`:
- `text` (default) — human readable
- `json` — structured data
- `paths` — file paths only, pipe-friendly

## Tips

- Use `--format json` when you need to parse output
- Use `knapper backlinks` to understand note context
- knapper does not do full-text search; use `rg` for that. knapper covers
  what `rg` cannot: the link graph, frontmatter, tasks, and link-preserving
  renames.
