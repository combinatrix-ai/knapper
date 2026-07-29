# knapper MCP server

Exposes knapper to an MCP client. It shells out to the `knapper` binary and
passes `--format json` through, so it uses the same interface as every other
caller rather than a private one that could drift, and needs nothing of
knapper but the executable on PATH.

## Installation

```bash
pip install mcp
```

knapper itself must be on PATH, or `KNAPPER_BIN` must point at it.

## Usage

### With Claude Desktop

```json
{
  "mcpServers": {
    "knapper": {
      "command": "python",
      "args": ["/path/to/knapper/knapper-mcp/server.py"],
      "env": {
        "KNAPPER_VAULT": "/path/to/your/vault"
      }
    }
  }
}
```

`KNAPPER_VAULT` sets the working directory; without it the server uses its own,
which is rarely what you want for a long-running client.

### Directly

```bash
cd /path/to/your/vault
python /path/to/knapper/knapper-mcp/server.py
```

## Tools

| Tool | Description |
|------|-------------|
| `knapper_query` | Filter notes by frontmatter, inline fields and link counts |
| `knapper_fields` | What `query` can filter on in this vault |
| `knapper_context` | Everything about one note in a single call |
| `knapper_tasks` | Find and filter tasks |
| `knapper_backlinks` | What references a file |
| `knapper_links` | Outgoing links from a file |
| `knapper_lint` | Vault health |
| `knapper_tags` | Tags with counts |
| `knapper_daily` | Create or get a daily note |
| `knapper_frontmatter_get` | Read frontmatter |
| `knapper_frontmatter_set` | Write one key — **edits the file** |
| `knapper_rename` | Rename a note and rewrite every inbound link — **edits the vault** |

The two write tools are marked because a model calling them changes the user's
notes. `knapper_rename` accepts `dry_run` to preview.

A knapper command that fails returns `{"error": "..."}` rather than dropping
the connection, so the model can see what went wrong and adjust.
