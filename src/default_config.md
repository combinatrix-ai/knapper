---
vault_path: .
template_engine: templater
# flavor: markdown        # markdown (default) | logseq
# exclude:                # subtrees no whole-vault command should read
#   - Archives/
#   - logs/
daily_notes:
  folder: Daily
  template: Templates/daily.md
  format: YYYY-MM-DD
templates:
  folder: Templates
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

## Tasks

- **done_date**: Whether to add completion date when marking tasks done
- **created_date**: Whether to add created date when creating new tasks
- **default_file**: Where to add new tasks (daily = today's daily note, inbox = inbox file,
  or a specific path)
