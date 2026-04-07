# Gateway Frontend Customization

The web gateway UI can be customized by writing files to the `.system/gateway/` workspace directory using `memory_write`. Changes take effect on page refresh.

## Quick Reference

### Branding & Layout

Write `.system/gateway/layout.json` to customize branding, tab order, and features:

```json
{
  "branding": {
    "title": "My AI Assistant",
    "colors": {
      "primary": "#e53e3e",
      "accent": "#dd6b20"
    }
  },
  "tabs": {
    "hidden": ["routines"],
    "default_tab": "chat"
  }
}
```

Example: `memory_write target=".system/gateway/layout.json" content='{"branding":{"title":"Acme AI","colors":{"primary":"#e53e3e"}}}' append=false`

### Custom CSS

Write `.system/gateway/custom.css` for style overrides:

Example: `memory_write target=".system/gateway/custom.css" content="body { --bg-primary: #1a1a2e; }" append=false`

Common CSS variables: `--color-primary`, `--color-accent`, `--bg-primary`, `--bg-secondary`, `--bg-tertiary`, `--text-primary`, `--text-secondary`, `--border`, `--success`, `--error`, `--warning`.

### Widgets

Create custom UI components in `.system/gateway/widgets/{id}/`. The directory name is the widget id — it matches the id field in `manifest.json` and the path segment in `GET /api/frontend/widget/{id}/{file}`.

- `manifest.json` — widget metadata (id, name, slot)
- `index.js` — widget code (calls `IronClaw.registerWidget()`)
- `style.css` — optional scoped styles (auto-prefixed with `[data-widget="{id}"]`)

**Slot:** only `tab` is currently mounted by the browser runtime — `IronClaw.registerWidget({ slot: "tab", ... })` adds a new tab to the tab bar. For inline rendering of structured data in chat messages, use `IronClaw.registerChatRenderer({ id, match, render })` instead. Additional slot names may be accepted by the server but will not be mounted anywhere in the UI yet.

## API Endpoints

- `GET /api/frontend/layout` — current layout config
- `PUT /api/frontend/layout` — update layout config
- `GET /api/frontend/widgets` — list installed widgets
- `GET /api/frontend/widget/{id}/{file}` — serve widget file
