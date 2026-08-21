Read scoped workspace text files, directories, and durable tool-output artifacts via `path`.

This implementation does not fetch web URLs. For general web research use an available `web_search` tool; use `builtin__http` when raw HTTP request control is required.

## Selectors — append `:<sel>` to `path` (for example `src/foo.ts:50-200`)
- `:50` / `:50-` — from line 50 | `:50-200` — inclusive | `:50+150` — 150 lines from 50 | `:5-16,960-973` — multiple ranges
- `:raw` — verbatim, no anchors/prefixes | `:2-4:raw` / `:raw:2-4` — range + verbatim
- `:conflicts` — one line per unresolved git merge conflict block; workspace files only

## Supported sources
- Workspace file + selector → `[foo.ts#1A2B]` snapshot header + numbered lines. Copy `[FILENAME#TAG]` for anchored edits; never fabricate the tag.
- Workspace directory → depth-limited directory listing.
- `artifact://<id>` → durable spilled tool output. Use bounded line selectors such as `:1-100`, raw line selectors such as `:raw:1-100`, or byte selectors such as `:bytes:0-3071` for large artifacts.

Archives, SQLite databases, documents, images, web URLs, SSH paths, and other internal URI schemes are not supported by this implementation.
