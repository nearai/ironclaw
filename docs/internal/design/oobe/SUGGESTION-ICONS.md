# Suggestion card icons — enum + schema addendum

Companion to [VISION-RECONCILIATION.md](VISION-RECONCILIATION.md). The brand-icon
vocabulary for the durable suggestions contract
([PR #7694](https://github.com/nearai/ironclaw/pull/7694), **now on `main`**) and
how the frontend consumes it. **This matches the shipped schema** — no longer a
proposal.

## The shipped card

```jsonc
{ "title": "...", "description": "...", "suggested_prompt": "...",
  "icon": "slack", "sources": ["Gmail", "Slack"] }
```

- `icon` — **required** brand-icon enum (values are exactly the list below). The
  model must choose one enum value; making it required forces it to reason about
  which tool a suggestion touches. **This is the authoritative icon source.**
- `sources` — 1–5 **concise human-readable tool names** ("Gmail", "Slack",
  "Web Search"), translated from the discovered extension/tool metadata. They are
  **display strings, not extension ids and not the icon source** — the
  generation prompt explicitly forbids exposing internal capability ids.

**This reversed a finding.** VISION-RECONCILIATION §3 said cards carried no tool
identity, which drove the connect-model conflict. With `icon`/`sources`, cards
*do* carry tool identity. The connect model **stays decoupled** (a catalog-driven
surface, §3.1); `icon` drives the card's brand mark and `sources` are available
for display, while per-card "Connect &lt;tool&gt;" is reopened as a review
question (VISION-RECONCILIATION §6.4).

## Icon comes straight from `icon`

`icon` is required and enum-constrained, so the frontend trusts it directly —
`resolveIconId` returns the `icon` value when it is a known enum member, else
`generic`. It does **not** derive the icon from `sources` (those are free-form
display names, not mappable ids), which also removes any icon-vs-sources drift.

## The enum (shipped)

`generic` is the **guaranteed-valid** value for tool-less suggestions
(e.g. "draft a project plan"). The frontend `BrandIconId` union equals this list
exactly, and a test pins every enum value to a renderable glyph.

| enum value | glyph |
|---|---|
| `gmail` | Gmail |
| `google_calendar` | Google Calendar |
| `google_docs` | Google Docs |
| `google_drive` | Google Drive |
| `google_sheets` | Google Sheets |
| `google_slides` | Google Slides |
| `github` | GitHub |
| `slack` | Slack |
| `notion` | Notion |
| `telegram` | Telegram |
| `web` | globe |
| `memory` | store |
| `generic` | sparkle |

### Shipped schema (`suggestions.output.json`)

```jsonc
"icon": {
  "type": "string",
  "enum": ["gmail","google_calendar","google_docs","google_drive",
           "google_sheets","google_slides","github","slack","notion",
           "telegram","web","memory","generic"]
},
"sources": {
  "type": "array", "minItems": 1, "maxItems": 5, "uniqueItems": true,
  "items": { "type": "string", "minLength": 1, "maxLength": 128 }
}
```

Both are in `required`, so every card carries an `icon` and ≥1 `source`.

### Shipped Rust contract

`RebornSuggestion` (in `ironclaw_product_contracts`) carries
`pub icon: String,` and `pub sources: Vec<String>,`.

## Frontend (PR #6994)

- `pages/chat/lib/brand-icons.tsx` — `BrandIconId`, `BRAND_ICON_IDS` (equal to
  the shipped enum), `resolveIconId(suggestion)` (trusts the required `icon`,
  falls back to `generic`), and `<BrandIcon>`. The colored marks reuse the
  license-clean inline SVGs already committed in the OOBE mockup;
  sheets/slides/web/memory/generic are neutral in-house glyphs. It lives in the
  lazy suggestion-surface chunk, so it adds **nothing** to the eager `/chat`
  bundle.
- `Suggestion` (in `suggestions-api.ts`) carries `icon` + `sources` (typed
  optional for defensive rendering); the card renders the resolved brand mark.

## Assets & sourcing

No web scraping. The eight primary marks were already committed to the repo
(the mockup); the gaps are in-house neutral glyphs. If a future brand needs a
mark, extend the in-repo set or take a path from a permissively-licensed set
(e.g. Simple Icons, CC0) — brand trademarks remain their owners'; these are
nominative-use marks for "this suggestion touches &lt;tool&gt;".
