#!/usr/bin/env python3
"""MCP server for knapper.

Invokes the `knapper` binary and passes its `--format json` output through,
rather than linking against it. That keeps this server to the same interface
every other caller uses, instead of a private one that could drift, and means
it needs nothing of knapper but the executable.

Set KNAPPER_BIN to point at a specific binary; otherwise `knapper` is taken
from PATH. Run the server from inside the vault, or set KNAPPER_VAULT.
"""

import json
import os
import shutil
import subprocess
import sys

try:
    from mcp.server import Server
    from mcp.server.stdio import stdio_server
    from mcp.types import TextContent, Tool
except ImportError:
    print("Error: mcp package not installed. Run: pip install mcp", file=sys.stderr)
    sys.exit(1)

server = Server("knapper")


def knapper(*args: str) -> str:
    """Run a knapper command and return its stdout.

    Failures come back as JSON rather than raising, so the model sees what went
    wrong instead of the connection dropping.
    """
    binary = os.environ.get("KNAPPER_BIN") or shutil.which("knapper")
    if not binary:
        return json.dumps({"error": "knapper not found on PATH. Set KNAPPER_BIN to its location."})

    try:
        result = subprocess.run(
            [binary, *args],
            cwd=os.environ.get("KNAPPER_VAULT") or os.getcwd(),
            capture_output=True,
            text=True,
            timeout=120,
        )
    except subprocess.TimeoutExpired:
        return json.dumps({"error": f"knapper {args[0]} timed out"})

    if result.returncode != 0:
        return json.dumps({"error": result.stderr.strip() or f"knapper exited {result.returncode}"})
    return result.stdout


def optional(arguments: dict, name: str) -> list[str]:
    value = arguments.get(name)
    return [str(value)] if value not in (None, "") else []


TOOLS = [
    Tool(
        name="knapper_query",
        description=(
            "Filter notes by frontmatter, inline fields and link counts. "
            "Operators: = != > < >= <= ~ (contains), a bare name for 'has this "
            "field', !name for 'does not'."
        ),
        inputSchema={
            "type": "object",
            "properties": {
                "where": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Filters, e.g. ['status=open', 'inlinks>3']",
                },
                "field": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Extra columns to return",
                },
                "from": {"type": "string", "description": "Only notes under this path"},
                "sort": {"type": "string", "description": "e.g. inlinks:desc"},
                "limit": {"type": "integer"},
            },
        },
    ),
    Tool(
        name="knapper_fields",
        description="List what query can filter on: computed fields, and what the notes declare.",
        inputSchema={"type": "object", "properties": {}},
    ),
    Tool(
        name="knapper_context",
        description=(
            "Everything about one note in a single call: content, links, "
            "backlinks, tags, headings, stats."
        ),
        inputSchema={
            "type": "object",
            "properties": {"file": {"type": "string"}},
            "required": ["file"],
        },
    ),
    Tool(
        name="knapper_tasks",
        description="Find and filter tasks across the vault. Dates are YYYY-MM-DD.",
        inputSchema={
            "type": "object",
            "properties": {
                "all": {"type": "boolean", "description": "Include completed tasks"},
                "overdue": {"type": "boolean"},
                "due_from": {"type": "string"},
                "due_to": {"type": "string"},
                "file": {"type": "string"},
                "tag": {"type": "string"},
            },
        },
    ),
    Tool(
        name="knapper_backlinks",
        description="What references this file.",
        inputSchema={
            "type": "object",
            "properties": {"file": {"type": "string"}},
            "required": ["file"],
        },
    ),
    Tool(
        name="knapper_links",
        description="Outgoing links from a file.",
        inputSchema={
            "type": "object",
            "properties": {"file": {"type": "string"}},
            "required": ["file"],
        },
    ),
    Tool(
        name="knapper_lint",
        description=(
            "Vault health: broken links, orphans, duplicate names, stubs, missing frontmatter."
        ),
        inputSchema={"type": "object", "properties": {}},
    ),
    Tool(
        name="knapper_tags",
        description="List tags in the vault with counts.",
        inputSchema={"type": "object", "properties": {}},
    ),
    Tool(
        name="knapper_daily",
        description="Create or get a daily note. Accepts today, yesterday, or YYYY-MM-DD.",
        inputSchema={"type": "object", "properties": {"date": {"type": "string"}}},
    ),
    Tool(
        name="knapper_frontmatter_get",
        description="Read frontmatter, or one key of it.",
        inputSchema={
            "type": "object",
            "properties": {"file": {"type": "string"}, "key": {"type": "string"}},
            "required": ["file"],
        },
    ),
    Tool(
        name="knapper_frontmatter_set",
        description="Write one frontmatter key. Edits the file.",
        inputSchema={
            "type": "object",
            "properties": {
                "file": {"type": "string"},
                "key": {"type": "string"},
                "value": {"type": "string"},
            },
            "required": ["file", "key", "value"],
        },
    ),
    Tool(
        name="knapper_rename",
        description=(
            "Rename a note and rewrite every inbound link, in both syntaxes. "
            "Edits the vault; pass dry_run to preview."
        ),
        inputSchema={
            "type": "object",
            "properties": {
                "old": {"type": "string"},
                "new": {"type": "string"},
                "dry_run": {"type": "boolean"},
            },
            "required": ["old", "new"],
        },
    ),
]


@server.list_tools()
async def list_tools() -> list[Tool]:
    return TOOLS


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    as_json = ["--format", "json"]

    if name == "knapper_query":
        args = ["query", *as_json]
        for where in arguments.get("where", []):
            args += ["--where", str(where)]
        for field in arguments.get("field", []):
            args += ["--field", str(field)]
        for flag, key in (("--from", "from"), ("--sort", "sort"), ("--limit", "limit")):
            if arguments.get(key) not in (None, ""):
                args += [flag, str(arguments[key])]
        result = knapper(*args)

    elif name == "knapper_fields":
        result = knapper("fields", *as_json)

    elif name == "knapper_context":
        result = knapper("context", str(arguments["file"]), *as_json)

    elif name == "knapper_tasks":
        args = ["tasks", *as_json]
        if arguments.get("all"):
            args.append("--all")
        if arguments.get("overdue"):
            args.append("--overdue")
        for flag, key in (
            ("--due-from", "due_from"),
            ("--due-to", "due_to"),
            ("--file", "file"),
            ("--tag", "tag"),
        ):
            if arguments.get(key) not in (None, ""):
                args += [flag, str(arguments[key])]
        result = knapper(*args)

    elif name == "knapper_backlinks":
        result = knapper("backlinks", str(arguments["file"]), *as_json)

    elif name == "knapper_links":
        result = knapper("links", str(arguments["file"]), *as_json)

    elif name == "knapper_lint":
        result = knapper("lint", *as_json)

    elif name == "knapper_tags":
        result = knapper("tags", *as_json)

    elif name == "knapper_daily":
        result = knapper("daily", *optional(arguments, "date"), *as_json)

    elif name == "knapper_frontmatter_get":
        result = knapper(
            "frontmatter",
            "get",
            str(arguments["file"]),
            *optional(arguments, "key"),
            *as_json,
        )

    elif name == "knapper_frontmatter_set":
        result = knapper(
            "frontmatter",
            "set",
            str(arguments["file"]),
            str(arguments["key"]),
            str(arguments["value"]),
        )

    elif name == "knapper_rename":
        args = ["rename", str(arguments["old"]), str(arguments["new"]), *as_json]
        if arguments.get("dry_run"):
            args.append("--dry-run")
        result = knapper(*args)

    else:
        result = json.dumps({"error": f"Unknown tool: {name}"})

    return [TextContent(type="text", text=result)]


async def main() -> None:
    async with stdio_server() as (read, write):
        await server.run(read, write, server.create_initialization_options())


if __name__ == "__main__":
    import asyncio

    asyncio.run(main())
