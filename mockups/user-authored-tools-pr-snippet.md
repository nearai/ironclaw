## User-authored tools UI preview

Design-system-faithful mockup of the phase-1 user-authored WASM tool flow in chat.

### 1. Intent recognized in normal chat

![Intent recognized](docs/images/user-authored-tools/01-intent-recognized.png)

### 2. Builder progress cards

![Builder progress](docs/images/user-authored-tools/02-builder-progress.png)

### 3. Manifest preview before install

![Manifest preview](docs/images/user-authored-tools/03-manifest-preview.png)

### 4. Existing install approval gate

![Install approval](docs/images/user-authored-tools/04-install-approval.png)

### 5. Post-install credential setup

![Post-install setup](docs/images/user-authored-tools/05-post-install-setup.png)

### 6. Tool ready in the same thread

![Tool ready](docs/images/user-authored-tools/06-tool-ready.png)

### Notes

- Uses the current IronClaw design system (existing chat shell, approval/auth card patterns, data-card style, tokens, spacing, and typography).
- Introduces one new visual primitive: the `tool-builder-card`.
- Scope shown here is phase 1 only: local / single-user / hosted single-tenant. Multi-tenant remains blocked until per-user ToolRegistry isolation exists.
