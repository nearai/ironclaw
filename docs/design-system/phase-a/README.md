# Phase A — Token foundation

First visible step of the design-system adoption tracked in
`/home/illia/.claude/plans/quiet-nibbling-moth.md`. Restructures
`theme.css` with Radix Themes-style 12-step scales anchored on
Defuse's OmniSwap whitelabel palette, introduces semantic role tokens,
a layered shadow system, extended radii + type scale, and swaps the
webfont from DM Sans → Geist (plus Geist Mono for monospace).

## What changed

- **Accent scale (`--accent-1..12` + `--accent-a1..12`)** — OmniSwap
  accent green, 12-step mint→forest gradient. Brand anchor at
  `--accent-9` = `#73e2a5` (dark) / `#0a5b38` (light). Replaces the
  single `--accent: #34d399` token.
- **Neutral scale (`--sand-1..12` + `--sand-a1..12`)** — OmniSwap
  gray, green-tinged warm neutrals.
- **Semantic role aliases** — `--color-bg`, `--color-surface`,
  `--color-surface-elevated`, `--color-border`, `--color-border-hover`,
  `--color-text`, `--color-text-muted`, `--color-text-subtle`,
  `--color-label`, `--color-warning(-foreground)`, `--accent-contrast`.
- **Layered shadow set** — `--shadow-paper`, `--shadow-widget`,
  `--shadow-card`, `--shadow-inset` replace the earlier single
  `--shadow`.
- **Radius** — adds `--radius-xl: 16px`, `--radius-2xl: 20px`,
  `--radius-4xl: 30px` (Defuse's big rounded-paper aesthetic).
- **Type scale** — adds `--text-4xl: 32px`, `--text-5xl: 48px`,
  `--text-6xl: 64px` + `--tracking-display`, `--tracking-6xl`.
- **Typography** — `--font-sans: 'Geist'`, `--font-mono: 'Geist Mono'`.
  Fallback chain preserves DM Sans / IBM Plex Mono so partial renders
  still look close to the target.
- **Legacy tokens preserved** — every existing name (`--bg`, `--text`,
  `--accent`, `--shadow-card`, …) is now an alias onto the new scales
  so the 20 split surface stylesheets under `styles/` require **zero
  changes**.

Everything else (spacing, motion, easings) is unchanged.

## Screenshots

Captured with `tests/e2e/scenarios/capture_design_surfaces.py` at
viewport 1280×800, Chromium. The `before/` set is the tree immediately
before Phase A; the `after/` set is the same tree with `theme.css`
restructured and the Geist font loaded.

### Dark mode

| Surface | Before | After |
|---|---|---|
| Chat (with seeded turn) | ![before](./before/dark-chat.png) | ![after](./after/dark-chat.png) |
| Workspace / memory | ![before](./before/dark-memory.png) | ![after](./after/dark-memory.png) |
| Jobs | ![before](./before/dark-jobs.png) | ![after](./after/dark-jobs.png) |
| Missions | ![before](./before/dark-missions.png) | ![after](./after/dark-missions.png) |
| Routines | ![before](./before/dark-routines.png) | ![after](./after/dark-routines.png) |
| Settings | ![before](./before/dark-settings.png) | ![after](./after/dark-settings.png) |
| Logs | ![before](./before/dark-logs.png) | ![after](./after/dark-logs.png) |
| Auth screen | ![before](./before/dark-auth.png) | ![after](./after/dark-auth.png) |

### Light mode (sampled surfaces)

| Surface | Before | After |
|---|---|---|
| Chat | ![before](./before/light-chat.png) | ![after](./after/light-chat.png) |
| Settings | ![before](./before/light-settings.png) | ![after](./after/light-settings.png) |
| Auth screen | ![before](./before/light-auth.png) | ![after](./after/light-auth.png) |

## Reproducing

```bash
# Baseline (before these changes)
git checkout staging
cargo build --no-default-features --features libsql --bin ironclaw
cd tests/e2e && source .venv/bin/activate
CAPTURE_SCREENSHOTS=1 SCREENSHOT_DIR=/tmp/before pytest scenarios/capture_design_surfaces.py

# After
git checkout feat/phase-a-tokens-defuse
cargo build --no-default-features --features libsql --bin ironclaw
cd tests/e2e && source .venv/bin/activate
CAPTURE_SCREENSHOTS=1 SCREENSHOT_DIR=/tmp/after  pytest scenarios/capture_design_surfaces.py
```

The capture scenario is opt-in (env var gated) and will be skipped by
the default `pytest scenarios/` run — it exists solely to produce
visual diffs for design-system PRs.
