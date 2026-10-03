# Compatibility

What knapper understands, ecosystem by ecosystem and convention by
convention. ✅ works · ⚠️ partial · ❌ not supported.

The ecosystem fixtures here are exercised by fixture vaults in
[`tests/fixtures/flavors/`](../tests/fixtures/flavors/) and pinned by
`tests/flavors.rs`. Nothing is listed on the strength of it probably
being fine.

---

## Obsidian

The canonical feature contracts, plugin versions, evidence, differences and
planned adapters are in [spec.md](../spec.md). This includes native links,
properties, daily notes, Tasks/Templater conventions and the embedded Dataview
DQL engine. Each has its own scope; none is a blanket full-Obsidian claim.

---

## Foam

- ✅ Wikilinks and markdown links side by side in one workspace
- ✅ YAML frontmatter
- ✅ `#tags`
- ⚠️ Foam-specific template placeholders are not expanded

## Dendron

- ✅ Dot-hierarchy filenames — `proj.knapper.design.md`
- ✅ Wikilinks over that hierarchy — `[[proj.knapper.design]]`
- ✅ Markdown links with the extension — `[d](proj.knapper.design.md)`
- ✅ **Markdown links without the extension** — `[d](proj.knapper.design)` resolves; a dot that is not a known extension is treated as part of the name
- ⚠️ Frontmatter `id:` is readable but is not used to resolve links, so a rename still relies on the name

## Zettelkasten

- ✅ ID-prefixed filenames — `202607281530 block model.md`
- ✅ Bare wikilinks between them
- ✅ Works equally with `zk`-style or Luhmann-style naming, since knapper only cares about filenames and links

## Plain markdown, and static site generators

Jekyll, Hugo, MkDocs, Quartz content directories.

- ✅ Markdown links only — no wikilinks needed anywhere
- ✅ YAML frontmatter
- ✅ `.md`, `.markdown` and `.mdx` are all scanned

---

## Logseq

Support is deliberately partial: knapper reads the **file-level projection**
of a Logseq graph and ignores block identity. See "Why not blocks" below.

Set `flavor: logseq` in `knapper.yaml` to turn on the ambiguous parts.

- ✅ **Page references** — `[[page]]`
- ✅ Outline files parse as ordinary markdown; nesting is preserved as text
- ✅ **Tasks** *(needs `flavor: logseq`)* — `TODO`, `DOING`, `NOW`, `LATER`, `WAITING`, `DONE`, `CANCELED`. `DOING`/`NOW` map onto the `wip` status.
- ✅ **`SCHEDULED:` / `DEADLINE:`** *(needs `flavor: logseq`)* — folded into the scheduled and due dates, so the normal `--due-from` style filters apply
- ✅ **Block references are ignored, not mis-read** — `((uuid))` no longer becomes a phantom link target
- ✅ **Macros** — `{{embed: ...}}` and friends no longer leak their contents into the link graph
- ✅ **Page and block properties** — `title::`, `tags::`, `id::` are read as inline fields. Logseq properties and Dataview inline fields are the same syntax carrying the same meaning, so they are handled as one concept.
- ✅ **`alias::`** — feeds the same alias resolution Obsidian's `aliases:` property does
- ❌ **Block identity** — `((uuid))` is skipped rather than modelled; see "Why not blocks"
- ⚠️ Journals under `journals/` are ordinary notes, not daily notes

Bare keywords need the flavor setting because `- TODO something` is
indistinguishable from prose. A markdown vault that happens to write that line
gains no tasks by default.

## Roam

Same posture as Logseq, for markdown exports.

Needs no configuration: Roam's syntax cannot occur in ordinary markdown, so
all of this is always on.

- ✅ **Page references** — `[[page]]`
- ✅ **Macros are ignored** — `{{[[query]]: ...}}`, `{{embed: ...}}`, `{{[[roam/js]]}}`. The `[[...]]` inside a macro names a command or a state, not a note, so counting it invents edges. These rarely surface as broken links, because names like `TODO` and `query` often exist as real pages, which makes the damage silent.
- ✅ **Nested brackets** — `[[[[Ideas]]]]` resolves to `Ideas`
- ✅ **Tasks** — `{{[[TODO]]}}` and `{{[[DONE]]}}` become open and done tasks
- ✅ **Block references are ignored** — `((uid))`, including the `[*](((uid)))` form an export writes
- ❌ **Block identity** — see "Why not blocks"
- ❌ **Attributes** — `attribute::`

### Why not blocks

Logseq and Roam make the *block* the addressable unit: every bullet can carry
an ID, be referenced from anywhere, and hold its own properties. knapper's
commands are all shaped `knapper VERB FILE`, and `backlinks FILE` has no
meaning when the referent is a bullet. Supporting it is not a parser change
but a second data model.

Block references also tend to be used far more sparingly than page
references, so the file-level projection keeps most of a graph's structure
while leaving out the part that would require a different data model.

---

## org-mode

`.org` files are scanned and parsed by a dedicated parser; org shares almost
no syntax with markdown, so it is not a variant of the markdown path.

- ✅ **Links** — `[[file:x.org][description]]`, `[[file:x.org]]`, `[[./x.org]]`, and bare `[[x.org]]`
  - ✅ `file:x.org::*Heading` resolves to the file
  - ✅ **`[[id:uuid]]`** — resolved against `:ID:` properties, as org-roam uses
  - ✅ **`[[*Heading]]`** — resolved against headings across *every* file. This is genuine global heading resolution, and org is the only ecosystem here that has it; Obsidian's `[[Note#Heading]]` is file-scoped.
  - ✅ `#custom-id` and external URLs are correctly not treated as note links
- ✅ **Keywords** — `#+TITLE:`, `#+FILETAGS:`, and any other `#+KEY:` becomes metadata
- ✅ **Property drawers** — `:PROPERTIES:` … `:END:`; the first drawer is file-level metadata
- ✅ **Aliases** — `#+ROAM_ALIASES:` and `:ROAM_ALIASES:`, including quoted multi-word entries
- ✅ **Tags** — `#+FILETAGS: :a:b:` and per-heading `:tag:`
- ✅ **Todo states** — `TODO`, `NEXT`, `WAITING`, `DONE`, `CANCELLED` and friends become tasks, with `[#A]` priorities
- ✅ **Planning** — `SCHEDULED:`, `DEADLINE:` and `CLOSED:` map onto the same date filters markdown tasks use, so `--due-from` and `--done-from` work across both formats
- ✅ **`#+BEGIN_SRC` blocks** and `=verbatim=` are excluded from link scanning
- ❌ Radio targets, `<<target>>`
- ❌ Custom todo keyword sets declared with `#+TODO:` — the built-in state names are assumed
- ⚠️ Writing is not supported: `tasks new` and `frontmatter set` still emit markdown

---

## Elsewhere

knapper reads notes. Formats that are not notes -- Obsidian Canvas and Bases,
Dendron schemas, Excalidraw drawings -- are left alone, and correctly ignored
as link targets rather than reported as broken.

Notion, Bear, Joplin and Tana are not markdown on disk. Export first, then
the result is one of the flavors above.
