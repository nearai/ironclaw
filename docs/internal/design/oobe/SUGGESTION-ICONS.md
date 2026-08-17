# Suggestion card icons — enum + schema addendum

Companion to [VISION-RECONCILIATION.md](VISION-RECONCILIATION.md). Defines the
brand-icon vocabulary for the durable suggestions contract
([PR #7694](https://github.com/nearai/ironclaw/pull/7694)) and how the frontend
consumes it.

## Context

The #7694 author is adding two fields to the generated card schema:

```jsonc
{ "title": "...", "description": "...", "suggested_prompt": "...",
  "icon": "<enum>", "source_ids": ["gmail", "slack"] }
```

- `source_ids` — the extension ids a suggestion relates to (from
  `crates/extensions/packages/`).
- `icon` — a required brand-icon enum; making it required forces the model to
  reason about which tool a suggestion touches.

**This reverses a finding.** VISION-RECONCILIATION §3 said cards carried no tool
identity, which drove the connect-model conflict. With `source_ids`/`icon`,
cards *do* carry tool identity. The connect model **stays decoupled** (a
catalog-driven surface, VISION-RECONCILIATION §3.1) — `source_ids` now also
powers the card's brand mark and a just-in-time in-thread `AuthRequired` prompt
on start — but per-card "Connect &lt;tool&gt;" is viable again if the review
wants it. Recorded as an open question, not a reversal of the decision.

## `icon` is derivable — constrain, don't duplicate

`icon` can be derived from `source_ids[0]`. Two model fields that must agree
invite drift (icon says slack, source_ids says gmail). Options:

1. **Drop `icon`**, derive frontend-side from `source_ids[0]`. Simplest; one
   source of truth. The frontend already does this (`iconIdForSource`).
2. **Keep `icon`** as the explicit "which tool is this about" signal, but
   **constrain the enum to the same namespace** so it can't meaningfully
   diverge, and have the frontend prefer `icon` then fall back to deriving from
   `source_ids` then to `generic` (what `resolveIconId` does today).

Either works with the shipped frontend — `resolveIconId` handles both. If `icon`
stays, it should be the enum below.

## The enum

Values mirror the extension-package namespace (snake_case), so cards, the
connect surface, and the extensions page can share one id → glyph table.
`generic` is the **required guaranteed-valid** value for tool-less suggestions
(e.g. "draft a project plan") — without it the model is forced to mislabel.

| enum value | source_ids it covers | glyph |
|---|---|---|
| `gmail` | `gmail` | Gmail |
| `google_calendar` | `google-calendar` | Google Calendar |
| `google_docs` | `google-docs` | Google Docs |
| `google_drive` | `google-drive` | Google Drive |
| `google_sheets` | `google-sheets` | Google Sheets |
| `google_slides` | `google-slides` | Google Slides |
| `github` | `github` | GitHub |
| `slack` | `slack` | Slack |
| `notion` | `notion-mcp` | Notion |
| `telegram` | `telegram` | Telegram |
| `web` | `web-access`, `web-app` | globe |
| `memory` | `mem0`, `memory-native` | store |
| `generic` | *(none / unknown / nearai-mcp)* | sparkle |

### JSON-schema block for `suggestions.output.v1.json`

```jsonc
"icon": {
  "type": "string",
  "enum": ["gmail","google_calendar","google_docs","google_drive",
           "google_sheets","google_slides","github","slack","notion",
           "telegram","web","memory","generic"]
},
"source_ids": {
  "type": "array",
  "items": { "type": "string" },
  "maxItems": 4
}
```

Add both to `required` if the field is meant to force the model's hand (with
`generic` always available, `icon` can safely be required).

### Suggested Rust contract type

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionIconId {
    Gmail, GoogleCalendar, GoogleDocs, GoogleDrive, GoogleSheets, GoogleSlides,
    Github, Slack, Notion, Telegram, Web, Memory, Generic,
}
```

On `RebornSuggestion`: `pub icon: SuggestionIconId,` and
`pub source_ids: Vec<String>,` (bounded, e.g. ≤4).

## Frontend (already built in this branch, PR #6994)

- `pages/chat/lib/brand-icons.tsx` — `BrandIconId`, `BRAND_ICON_IDS`,
  `iconIdForSource(sourceId)`, `resolveIconId(suggestion)`, and `<BrandIcon>`.
  The colored marks reuse the license-clean inline SVGs already committed in
  the OOBE mockup; sheets/slides/web/memory/generic are neutral in-house glyphs.
  It lives in the lazy suggestion-surface chunk, so it adds **nothing** to the
  eager `/chat` bundle.
- `Suggestion` (in `suggestions-api.ts`) carries optional `icon` +
  `source_ids`, and the card renders the resolved brand mark. All fields are
  optional and everything degrades to `generic`, so the card is correct **now**,
  before the backend field lands.

## Assets & sourcing

No web scraping. The eight primary marks were already committed to the repo
(the mockup); the gaps are in-house neutral glyphs. If a future brand needs a
mark, extend the in-repo set or take a path from a permissively-licensed set
(e.g. Simple Icons, CC0) — brand trademarks remain their owners'; these are
nominative-use marks for "this suggestion touches &lt;tool&gt;".
