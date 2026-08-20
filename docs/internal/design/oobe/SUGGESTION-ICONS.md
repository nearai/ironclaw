# Suggestion card icons — semantic enum contract

Companion to [VISION-RECONCILIATION.md](VISION-RECONCILIATION.md). This document
defines the shipped suggestion icon contract and its frontend rendering.

## Card shape

```jsonc
{ "title": "...", "description": "...", "suggested_prompt": "...",
  "icon": "messaging", "sources": ["Team chat"] }
```

- `icon` is a required, provider-neutral task category. It controls only the
  card glyph; it is not an extension, vendor, or capability identity.
- `sources` contains one to five concise, human-readable provenance labels
  translated from discovered extension or tool metadata. Sources are display
  strings, not extension IDs, and the frontend never derives an icon or setup
  route from them.

This separation keeps generic suggestion rendering independent of the set of
installed extensions. Extension-owned presentation metadata can be introduced
through the extension catalog later without embedding concrete identities in
generic chat code.

## Semantic vocabulary

The schema and frontend use the same ordered vocabulary:

| enum value | task concept |
|---|---|
| `email` | email work |
| `calendar` | scheduling |
| `document` | documents |
| `storage` | files and storage |
| `spreadsheet` | tables and spreadsheets |
| `presentation` | slides and presentations |
| `code` | source code |
| `messaging` | conversations and messages |
| `notes` | notes and writing |
| `web` | web research |
| `memory` | retained context |
| `generic` | uncategorized work and fallback |

`generic` is the guaranteed fallback. The model must choose a schema member,
but the frontend also maps unknown, missing, and legacy persisted values to
`generic` so cards always remain renderable.

### Schema (`suggestions.output.json`)

```jsonc
"icon": {
  "type": "string",
  "enum": ["email", "calendar", "document", "storage", "spreadsheet",
           "presentation", "code", "messaging", "notes", "web", "memory",
           "generic"]
},
"sources": {
  "type": "array", "minItems": 1, "maxItems": 5, "uniqueItems": true,
  "items": { "type": "string", "minLength": 1, "maxLength": 128 }
}
```

Both fields are required. The Rust wire contract intentionally stores `icon`
as a string so schema evolution does not require a persistence migration.

## Frontend

`pages/chat/lib/suggestion-icons.tsx` owns `SuggestionIconId`,
`SUGGESTION_ICON_IDS`, `resolveIconId`, and `<SuggestionIcon>`. It maps semantic
categories to the shared design-system glyphs and lives in the lazy suggestion
surface chunk. `Suggestion` types `icon` and `sources` as optional only for
defensive rendering of incomplete or older records.

## Compatibility

Suggestions persisted with the retired concrete-brand values still load and
remain startable. Their unknown icon value renders the neutral `generic` glyph;
no rows are rewritten or deleted.
