# Skills Tab - Web UI Design

## Goal

Add a Skills tab to the IronClaw web gateway that lets users browse installed skills, search ClawHub for new skills, and install/remove skills -- all from the browser.

## Scope

**Frontend only.** The REST API endpoints already exist:

| Method | Endpoint | Purpose |
|--------|----------|---------|
| GET | `/api/skills` | List installed skills |
| POST | `/api/skills/search` | Search ClawHub + local |
| POST | `/api/skills/install` | Install (requires `X-Confirm-Action: true`) |
| DELETE | `/api/skills/{name}` | Remove (requires `X-Confirm-Action: true`) |

No Rust changes needed.

## Layout

Three sections inside the tab panel:

### 1. Search ClawHub

A search input at the top. On submit, calls `POST /api/skills/search` and renders catalog results as dashed-border cards (matching the "available extension" pattern). Cards that match an already-installed skill show "Installed" instead of an Install button.

Staggered fade-in animation on search results for polish.

### 2. Installed Skills

Grid of cards for all locally loaded skills. Each card shows:
- **Name** (bold, `.ext-name` style)
- **Trust badge**: "Trusted" (green) or "Installed" (blue) -- small pill
- **Version** (small, secondary text)
- **Description** (`.ext-desc` style)
- **Activation keywords** as small tags (`.ext-keywords` style)
- **Remove button** -- only for registry-installed skills (trust=Installed), not user-placed trusted skills

### 3. Install by URL

A small form matching the WASM install form pattern:
- Name input
- URL input (HTTPS)
- Install button

## Visual Design

Reuses existing `.ext-card`, `.extensions-list`, `.extensions-section`, `.btn-ext` classes. New CSS limited to:
- `.skill-trust` badge pill (green for Trusted, blue for Installed)
- `.skill-version` small version label
- Staggered `@keyframes skillFadeIn` for search results
- `.skill-search-box` for the search input styling

## Files Modified

- `src/channels/web/static/index.html` -- Add Skills tab button + panel markup
- `src/channels/web/static/app.js` -- Add `loadSkills()`, `searchClawHub()`, `installSkill()`, `removeSkill()`, render functions, wire into `switchTab()`
- `src/channels/web/static/style.css` -- Trust badge styles, search box, fade-in animation

## Decisions

- Reuse ext-card classes rather than creating a parallel card system
- Trust badge differentiates skills from extensions visually
- Confirmation uses `window.confirm()` dialog matching `removeExtension()` pattern
- Search is manual (button/enter) not live-as-you-type to avoid hammering ClawHub
