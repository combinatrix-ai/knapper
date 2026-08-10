---
vault_path: .
exclude:
  - logs
tasks:
  statuses:
    forward:
      char: ">"
      closed: true
      date_format: "➡️ YYYY-MM-DD"
    cancel:
      date_format: "🚫 YYYY-MM-DD"
---
