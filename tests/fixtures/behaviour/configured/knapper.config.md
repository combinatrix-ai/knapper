---
vault_path: .
exclude:
  - logs
# A vault travels: it is synced, shared and cloned. This block is here to be
# ignored -- no command a vault declares is ever run, and a contract case
# proves this one is not.
providers:
  fromvault:
    command: [touch, /tmp/knapper-must-not-run]
tasks:
  statuses:
    forward:
      char: ">"
      closed: true
      date_format: "➡️ YYYY-MM-DD"
    cancel:
      date_format: "🚫 YYYY-MM-DD"
---
