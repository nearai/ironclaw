# Reborn TUI

A [ratatui](https://ratatui.rs/) terminal client for `ironclaw-reborn`. It is a
thin protocol client of `ironclaw-reborn serve`'s WebChat v2 HTTP + SSE API
(`/api/webchat/v2/*`) — the same API the browser WebChat v2 UI talks to — so it
supports chatting in threads, resolving gates (approvals and auth/OAuth
prompts), browsing and pausing/resuming automations, and switching the active
LLM provider/model, all from a terminal.

## Quick start

**First run:** provision an LLM provider and the WebChat bearer token:

```sh
ironclaw-reborn onboard
```

This writes the WebChat v2 bearer token to `<reborn_home>/webui-token`
(`0600` permissions) so the TUI (and a service-installed `serve`) always has
a token to read.

**Run the TUI:**

```sh
ironclaw-reborn tui
```

It resolves a base URL the same way `serve` does (`--base-url` flag > `[webui]`
config > compiled default `http://127.0.0.1:3000`), and:

- If a `serve` is already reachable there, it attaches to it directly.
- If not, it **auto-spawns its own `serve` child process** (stdout/stderr
  redirected to `<reborn_home>/tui-serve.log` — never the terminal), waits up
  to 15s for it to become healthy, and **reaps the child on exit**.

To attach to a specific already-running `serve` instead:

```sh
ironclaw-reborn tui --base-url http://host:port
```

**Token:** resolved with the same precedence `serve` uses — the
`IRONCLAW_REBORN_WEBUI_TOKEN` env var (or whatever `[webui].env_token_var`
renames it to) if set and non-empty, else the `onboard`-provisioned
`<reborn_home>/webui-token` file. Neither present is a fail-closed error
naming both. The token doubles as the session-signing key, so it must be at
least 32 bytes; for an ad-hoc run against a different token:

```sh
IRONCLAW_REBORN_WEBUI_TOKEN=$(openssl rand -hex 32) ironclaw-reborn tui
```

**Build note:** the `tui` subcommand is compiled only behind the
`webui-v2-beta` cargo feature (same gate as `serve`):

```sh
cargo build --features webui-v2-beta
```

Release bundles that ship `serve` already include this feature.

## Keybindings

Quit (`Ctrl+C` / `Ctrl+D`) works from any screen, including inside a modal or
a pending gate.

**Global (composer focus, no modal or gate open)**

| Key | Action |
|---|---|
| `Ctrl+X` | Open the threads modal |
| `Ctrl+A` | Open the automations modal |
| `Ctrl+P` / `Ctrl+L` | Open the provider modal |
| `Ctrl+C` / `Ctrl+D` | Quit |
| `/exit` + `Enter` | Quit (typed as the message text) |

**Chat / transcript**

| Key | Action |
|---|---|
| `Enter` | Send the composed message |
| `Backspace` | Delete the last typed character |
| `PageUp` / `PageDown` | Scroll the transcript by one page |
| `Home` | Jump to the top of the transcript |
| `End` | Resume following the live tail |
| `Esc` | Cancel the in-flight run (only while a turn is running) |

**Threads modal (`Ctrl+X`)**

| Key | Action |
|---|---|
| `Up` / `Down` | Move selection (row 0 is the pinned "+ new" entry) |
| `Enter` | Create a new thread (row 0) or switch to the selected thread |
| `d`, `d` | Delete the selected thread (second `d` confirms) |
| `Esc` | Close the modal |

**Automations modal (`Ctrl+A`)**

| Key | Action |
|---|---|
| `Up` / `Down` | Move selection |
| `Space` | Pause (if active/scheduled) or resume (if paused) the selected automation |
| `r`, type name, `Enter` | Rename the selected automation (`Esc` cancels the rename draft) |
| `Enter` | Open the selected automation's run thread |
| `Esc` | Close the modal |

**Provider modal (`Ctrl+P` / `Ctrl+L`)**

| Key | Action |
|---|---|
| `Up` / `Down` | Move selection |
| `Enter` | Providers level: list that provider's models. Models level: set the model active and run a connection test. |
| `Esc` | Step back one level (Models → Providers); from Providers, closes the modal |

**Gate / approval / auth prompt**

| Key | Action |
|---|---|
| `a` | Allow |
| `A` | Allow always (only shown when the prompt offers it) |
| `d` | Deny |
| `Esc` | Cancel (resolves the gate server-side as declined, same as `d`) |
| `o` | Open the authorization URL in the OS browser (auth prompts with a URL) |
| `t`, type token, `Enter` | Enter a manual token (auth prompts); `Esc` exits the token entry back to the gate prompt without resolving it |

## Automations & held approvals

The automations panel (`Ctrl+A`) lists every automation, including completed
ones, and shows a hold badge next to any automation currently parked on an
approval, an auth/OAuth prompt, or an in-progress run. Pressing `Enter` on a
held automation opens its run thread; the pending approval or auth prompt
renders in that thread's gate zone, where you resolve it inline with the same
keys (`a`/`A`/`d`/`t`/`o`/`Esc`) as any other gate — the identical flow to
resolving it from the browser UI.

## Running as a background service

`ironclaw-reborn service` installs `serve` as an OS-managed background
process (launchd on macOS, systemd on Linux):

```sh
ironclaw-reborn service install     # write and enable the unit
ironclaw-reborn service start       # start it
ironclaw-reborn service status      # check whether it's running
ironclaw-reborn service stop        # stop it
ironclaw-reborn service uninstall   # remove the unit
```

The service runs `serve` continuously in the background. Once it's running,
launch `ironclaw-reborn tui` (or open the browser WebChat UI) to attach to it
— no auto-spawn needed, since `serve` is already reachable.

## Relationship to the web UI

The TUI mirrors the WebChat v2 browser UI's actions — chat, thread
management, gate/auth resolution, automation pause/resume, and provider/model
switching all go through the same HTTP + SSE API. Two gaps to know about so
they aren't a surprise:

- **No extensions configuration** — the TUI has no surface for installing or
  configuring extensions; use the browser UI for that.
- **No provider add/edit** — the provider modal only lets you pick among
  already-configured providers and their models; adding a new provider or
  editing its config still requires the browser UI or the config file.

Automation create/edit/delete are also out of scope for the TUI by design
(there's no create endpoint server-side, and delete was deliberately left out
to keep the panel read-mostly) — this matches the browser UI's own
automations panel scope, not a TUI-specific gap.
