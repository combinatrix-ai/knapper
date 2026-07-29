# Flavor fixtures

One tiny vault per note-taking flavor, used to keep knapper's compatibility
claims honest.

**The rule: a flavor may only be named in the README or in
`docs/COMPATIBILITY.md` once it has a fixture here and a row in
`tests/flavors.rs`.**
Claims that are not exercised by a test rot silently — this directory exists
so that "works with Foam" is a passing test rather than a hope.

## Current support

The user-facing enumeration lives in [`docs/COMPATIBILITY.md`](../../../docs/COMPATIBILITY.md),
broken down by ecosystem and by plugin. The table below is the coarse view that
`tests/flavors.rs` actually asserts, so any change in behaviour shows up as
a diff rather than as a surprise.

| Flavor | Links | Tasks | Notes |
|---|---|---|---|
| `obsidian` | ✅ | ✅ | Wikilinks, `aliases:`, `#heading`, embeds, link-valued properties, `- [ ]` with the full emoji set, Unicode and nested tags, and code/comments correctly excluded |
| `foam` | ✅ | — | Wikilinks and markdown links side by side; same shape as Obsidian |
| `zettelkasten` | ✅ | — | ID-prefixed filenames, bare wikilinks |
| `markdown-plain` | ✅ | — | Markdown links only, including anchors, percent-encoding, and images/externals correctly ignored |
| `dendron` | ✅ | — | Dot-hierarchy filenames, and all three link forms including the extensionless `[d](proj.knapper.design)` |
| `dataview` | ✅ | — | All three inline field forms are read, and a wikilink value reaches the link graph as a typed link |
| `logseq` | ✅ | ✅ | Fixture sets `flavor: logseq`. Keywords become tasks, `SCHEDULED:` becomes a date, `property::` is read as an inline field, and `((uuid))` is ignored |
| `roam` | ✅ | ✅ | No config needed. Macros and `((uid))` are ignored, `[[[[x]]]]` parses, `{{[[TODO]]}}` is a task |
| `orgmode` | ✅ | ✅ | Own parser: `[[file:x.org][d]]`, `[[id:uuid]]`, global `[[*Heading]]`, `:PROPERTIES:` drawers, `#+FILETAGS:`, TODO/DONE with SCHEDULED/DEADLINE/CLOSED |

One directory here is not a flavor. `bare/` has no `knapper.config.md` and
belongs to no ecosystem: it is what any vault looks like the moment before
`knapper init`. Every other fixture ships a config, so that state was never
exercised — and both implementations turned out to be wrong in it, the Rust
one refusing to run `init` at all. It has contract cases like the rest, which
is why it lives here rather than in a test file.

## Intent per flavor

- **obsidian** — the reference flavor; exercises every supported construct.
- **foam** — proves both link syntaxes coexist in one vault.
- **zettelkasten** — filenames that are IDs, not titles.
- **markdown-plain** — a vault with no wikilinks at all, which is the case
  that matters for not being tied to Obsidian.
- **dendron** — dot-hierarchy naming, where a filename contains dots that are
  not extensions.
- **dataview** — bracketed inline fields, `[key:: value]`, including a typed
  link. This is how relations are written in practice in Obsidian vaults.
- **logseq** / **roam** — outliner exports. Every line is a bullet, properties
  use `::`, tasks are keywords, and references can point at a block rather
  than a file. Support here is deliberately partial: knapper reads the
  file-level projection and ignores block identity.
- **orgmode** — a separate syntax for links, properties, tags and todos, including the global heading resolution no markdown flavor has.

## Adding a flavor

1. Add a directory with a `knapper.config.md` and the smallest set of notes
   that exercises the constructs the flavor actually uses.
2. Add a row to `EXPECTED` in `tests/flavors.rs`.
3. Only then mention the flavor in user-facing docs.
