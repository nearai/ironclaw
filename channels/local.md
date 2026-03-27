---
title: "Local"
description: "Use IronClaw locally via terminal or browser"
---

By default, IronClaw provides two local interfaces for chatting with your agent:

- **Terminal UI (TUI):** chat directly in your terminal
- **Web Gateway:** chat in your browser over a local HTTP server

<Note>
If you haven't set up your agent yet, follow our [Quickstart guide](../quickstart)
</Note>

---

## Terminal UI

Simply run `ironclaw` and the TUI will launch in your terminal. Use the keyboard shortcuts below to navigate and chat with your agent.
| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Shift+Enter` | New line in composer |
| `Ctrl+C` | Quit |
| `Ctrl+L` | Clear screen |
| `Tab` | Focus next element |
| `Esc` | Cancel or back |
| `Up/Down` | Scroll history |

### Configuration

| Option | Default | Description |
|--------|---------|-------------|
| `CLI_ENABLED` | `true` | Enable or disable the Terminal UI |


---

## Web Gateway

| Option | Default | Description |
|--------|---------|-------------|
| `GATEWAY_HOST` | `127.0.0.1` | Host interface for the Web Gateway |
| `GATEWAY_PORT` | `3000` | Port used by the Web Gateway |
| `GATEWAY_ENABLED` | `true` | Enable or disable the Web Gateway |
| `GATEWAY_AUTH_TOKEN` | auto-generated | Auth token required to open the Web UI |

### Authentication

By default, IronClaw generates an auth token at startup and prints it in logs. To use a stable token across restarts:

```bash
export GATEWAY_AUTH_TOKEN="your-secure-token-here"
```

Generate one:

```bash
openssl rand -hex 32
```

### API endpoints

The Web Gateway also exposes local endpoints:

| Endpoint | Description |
|----------|-------------|
| `GET /api/status` | Server status |
| `POST /api/chat` | Send message |
| `GET /api/jobs` | List jobs |
| `GET /api/memory` | Search memory |

### Network access

Use localhost-only access (recommended):

```bash
export GATEWAY_HOST=127.0.0.1
```

Use LAN access:

```bash
export GATEWAY_HOST=0.0.0.0
```

<Warning>
When using `0.0.0.0`, use a strong auth token and place the service behind HTTPS/reverse proxy before exposing it outside your local network.
</Warning>

---

## Troubleshooting

<AccordionGroup>
  <Accordion title="Terminal display issues">
    - Ensure your terminal supports Unicode and 256 colors
    - Set `TERM=xterm-256color`
    - Restart the terminal session
  </Accordion>

  <Accordion title="Terminal input issues">
    - Check terminal focus
    - Run `reset`
    - Disable conflicting terminal mouse mode
  </Accordion>

  <Accordion title="Web UI connection refused">
    - Verify `ironclaw run` is active
    - Check `GATEWAY_PORT` value
    - Confirm host and firewall settings
  </Accordion>

  <Accordion title="Web UI token rejected">
    - Copy token exactly from startup logs
    - Remove trailing spaces
    - Set a persistent `GATEWAY_AUTH_TOKEN`
  </Accordion>

  <Accordion title="WebSocket disconnects">
    - Check local network/proxy stability
    - Verify reverse proxy supports WebSocket upgrades
    - Inspect browser console logs
  </Accordion>
</AccordionGroup>
