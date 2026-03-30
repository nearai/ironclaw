# Frontend Customization

The web gateway UI can be customized by writing files to the `frontend/` workspace directory using `memory_write`. Changes take effect on page refresh.

## Quick Reference

### Branding & Layout

Write `frontend/layout.json` to customize branding, tab order, and features:

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

Example: `memory_write path="frontend/layout.json" content='{"branding":{"title":"Acme AI","colors":{"primary":"#e53e3e"}}}' append=false`

### Custom CSS

Write `frontend/custom.css` for style overrides:

Example: `memory_write path="frontend/custom.css" content="body { --bg-primary: #1a1a2e; }" append=false`

Common CSS variables: `--color-primary`, `--color-accent`, `--bg-primary`, `--bg-secondary`, `--bg-tertiary`, `--text-primary`, `--text-secondary`, `--border`, `--success`, `--error`, `--warning`.

### Widgets

Create custom UI components in `frontend/widgets/{name}/`:
- `manifest.json` — widget metadata (id, name, slot)
- `index.js` — widget code (calls `IronClaw.registerWidget()`)
- `style.css` — optional scoped styles

Slots: `tab`, `chat_header`, `chat_footer`, `sidebar`, `settings_section`.

## API Endpoints

- `GET /api/frontend/layout` — current layout config
- `PUT /api/frontend/layout` — update layout config
- `GET /api/frontend/widgets` — list installed widgets
- `GET /api/frontend/widget/{id}/{file}` — serve widget file
