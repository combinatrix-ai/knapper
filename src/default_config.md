---
vault_path: .
template_engine: templater
# flavor: markdown        # markdown (default) | logseq
# exclude:                # subtrees no whole-vault command should read
#   - Archives/
#   - logs/
# ignore_links:           # link targets that are meant to stay unresolved
#   - Daily Tasks         # matched whole, case-insensitively, never as a
#   - Archive/Old Index   # substring; write the path to ignore a path link
daily_notes:
  folder: Daily
  template: Templates/daily.md
  format: YYYY-MM-DD
tasks:
  done_date: true                    # Add completion date when marking done
  done_date_format: "✅ YYYY-MM-DD"  # Format for completion date
  created_date: true                 # Add created date when creating new tasks
  created_date_format: "➕ YYYY-MM-DD"  # Format for created date
  default_file: daily                # daily | inbox | path
  inbox: Inbox/Tasks.md              # Inbox file path
  # Built-in statuses: open " ", done "x", wip "/", cancel "-".
  # Override fields on existing statuses or add custom ones. Example:
  # statuses:
  #   cancel:
  #     date_format: "🚫 YYYY-MM-DD"  # override the default ❌ marker
  #   forward:
  #     char: ">"
  #     closed: true
  #     date_format: "➡️ YYYY-MM-DD"
---

# Knapper Configuration

This is the configuration file for knapper CLI.

## Links

- **ignore_links**: Link targets this vault never intends to resolve. They stop
  being reported by `knapper lint`, `knapper broken-links` and `query`'s
  `broken` field. An entry matches a whole link target, case-insensitively,
  and may be written the way the link is written in a note (`[[Habits]]`,
  `Habits.md` and `Habits` are the same entry).

Link targets are discovered from every visible file, including notes below an
`exclude` entry and non-note files such as `.pdf`, `.txt` and `.json` leaves.
Excluded files and leaves are never parsed as source notes and do not appear in
`query`, `orphans` or `hubs`; they can still be valid destinations of links from
an included note. A path-qualified target is first tried as a vault-root path,
then relative to the referring note, with `..` traversal that would leave the
vault rejected. Bare note links retain basename and alias resolution.

## Tags

`[[X]]` and `#X` are different references and neither is configuration.
`[[X]]` is a hard note reference: it names a note, it is an edge in the graph,
and a missing target is a broken link. `#X` is a soft topic reference: it
labels a note, requires no note to exist, is never broken, and is never a node
in the graph, an orphan or a hub.

A tag is still navigable. `knapper backlinks '#X'` and `knapper context '#X'`
resolve a tag as a virtual subject — the notes and lines that carry it — and a
leading `#` is the only thing that asks for that. `knapper demote X` rewrites
the exact `[[X]]` into `#X` for a vault whose wikilinks were only ever labels;
`--dry-run` shows the plan first. If a target is meant to stay a link and stay
unresolved, `ignore_links` above is the other answer.

## Tasks

- **done_date**: Whether to add completion date when marking tasks done
- **created_date**: Whether to add created date when creating new tasks
- **default_file**: Where to add new tasks (daily = today's daily note, inbox = inbox file,
  or a specific path)
