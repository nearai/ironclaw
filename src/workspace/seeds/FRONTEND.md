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

**CSS scoping caveat:** Widget CSS is scoped via a brace-counting text transform, not a full CSS parser. Braces inside CSS comments (`/* } */`) or string literals (`content: "{"`) will confuse the scoper and produce malformed output. Avoid `{` / `}` in comments and string values — use Unicode escapes (`\7B` / `\7D`) if you need literal braces in `content:` properties.

**Binary assets:** Widget files are served through the workspace text layer (`Workspace::read()`), which returns UTF-8 strings. Text-format assets (JS, CSS, JSON, SVG) work correctly. Binary assets (PNG, WOFF2, TTF, etc.) will be corrupted — host them externally or Base64-encode them into CSS/JS until a binary workspace read path is available.

**Slot:** only `tab` is currently mounted by the browser runtime — `IronClaw.registerWidget({ slot: "tab", ... })` adds a new tab to the tab bar. For inline rendering of structured data in chat messages, use `IronClaw.registerChatRenderer({ id, match, render })` instead. Additional slot names may be accepted by the server but will not be mounted anywhere in the UI yet.

## API Endpoints

- `GET /api/frontend/layout` — current layout config
- `PUT /api/frontend/layout` — update layout config
- `GET /api/frontend/widgets` — list installed widgets
- `GET /api/frontend/widget/{id}/{file}` — serve widget file

## Security model

**Widgets run inside the gateway page with full session authority.** A widget's `index.js` is loaded as an inline ES module (under a per-response CSP nonce) into the same browser document as the rest of the gateway UI, which means:

- Widgets can call any gateway API the logged-in user can call. The runtime exposes `IronClaw.api.fetch(...)`, which forwards the user's bearer token automatically — there is no per-widget capability sandbox.
- Widgets can read and modify the same DOM as the built-in tabs, including the chat input, message history, and any other widget on the page.
- Widget CSS is scoped to `[data-widget="{id}"]`, but JavaScript is **not** sandboxed. A widget can reach out of its tab panel via `document.querySelector` and touch global state.

This is acceptable because the trust boundary lives one layer up: widgets are loaded from `.system/gateway/widgets/` in the **workspace**, which is itself a privileged store accessible only via authenticated `memory_write` calls. Anything that can write a widget file can already drive the agent directly. The widget runtime is therefore an extension surface for the operator, not a sandbox for untrusted code — treat installing a widget the same way you would treat running a script under your own user.

**Practical implications:**

- Do not install widgets from sources you wouldn't paste into a terminal as a shell script.
- A widget that calls `IronClaw.api.fetch('/api/memory/write', ...)` can mutate any workspace file, including its own source — review widget code before installing it.
- The widget identifier validator (`is_safe_widget_id` in `crates/ironclaw_gateway/src/layout.rs`, applied to discovery, serving, and `manifest.id`) and the `manifest.id == directory_name` check protect against widget-to-widget confusion, not against malicious-but-well-formed widgets.
- Defense-in-depth XSS protection (`escape_tag_close` for inline script/style breakouts, `is_safe_css_color` for branding values, CSP nonces) keeps a hostile `layout.json` field from breaking the page chrome — but those defenses do not apply to widget JS, which is intentionally given full execution authority.

The gateway CSP allows `'nonce-…'` only for `<script>` tags emitted by `assemble_index`, so a widget cannot inject *additional* inline scripts at runtime, and it cannot use `eval()`, `new Function()`, or string-form `setTimeout` / `setInterval` either — the gateway CSP does **not** include `'unsafe-eval'`. What a widget *can* still do, without tripping the CSP, is plenty: it can call `IronClaw.api.fetch` against any same-origin endpoint, mutate the entire DOM, attach event listeners on the chat input, and pull additional ES modules via dynamic `import()` from any origin allowed by the gateway's `script-src` (currently `'self'`, jsDelivr, cdnjs, esm.sh). The CSP narrows the *shape* of attacks a widget can mount, not the blast radius. Operators who want stricter isolation should run untrusted UI code in an `<iframe sandbox>` mounted by a *trusted* widget rather than registering it directly.
