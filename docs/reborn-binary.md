# `ironclaw-reborn` standalone binary

`ironclaw-reborn` is the standalone executable boundary for Reborn. It is separate from the current `ironclaw` binary so Reborn boot, config, state, and runtime composition can evolve without accidentally invoking v1 runtime paths.

This binary is available as the workspace package `ironclaw_reborn_cli` and builds the executable named `ironclaw-reborn`.

## Current status

`ironclaw-reborn` is an early operator/testing surface, not the default IronClaw runtime.

It currently supports:

```bash
ironclaw-reborn --help
ironclaw-reborn channels list
ironclaw-reborn channels list --json
ironclaw-reborn channels list --verbose
ironclaw-reborn completion --shell bash
ironclaw-reborn completion --shell zsh
ironclaw-reborn config path
ironclaw-reborn doctor
ironclaw-reborn extension search github
ironclaw-reborn extension search github --json
ironclaw-reborn extension install github-mcp
ironclaw-reborn extension activate github-mcp
ironclaw-reborn extension remove github-mcp
ironclaw-reborn hooks list
ironclaw-reborn hooks list --json
ironclaw-reborn hooks list --verbose
ironclaw-reborn logs
ironclaw-reborn logs --json
ironclaw-reborn logs --verbose
ironclaw-reborn models list
ironclaw-reborn models list --json
ironclaw-reborn models status
ironclaw-reborn models status --json
ironclaw-reborn models set-provider openai --model gpt-5-mini
ironclaw-reborn onboard
ironclaw-reborn onboard --dry-run
ironclaw-reborn onboard --force
ironclaw-reborn onboard --import-history   # flag parsed, but history import not wired yet
ironclaw-reborn profile list
ironclaw-reborn profile list --json
ironclaw-reborn repl
ironclaw-reborn run
ironclaw-reborn run --confirm-host-access
ironclaw-reborn serve                      # only compiled in with --features webui-v2-beta
ironclaw-reborn serve --confirm-host-access
ironclaw-reborn skills list
ironclaw-reborn skills list --json
ironclaw-reborn skills list --verbose
```

The `traces` command tree is a contributor-only trace client; see
`crates/ironclaw_reborn_cli/src/commands/traces/` for its subcommands.

It intentionally does not yet support:

- replacing `ironclaw` behavior;
- daemon/service installation;
- v1 config, DB, settings, or secrets migration;
- production extension/tool execution;
- long-lived Reborn runtime services.

The WebChat v2 web UI **is** supported through `serve`, but only when the
binary is built with `--features webui-v2-beta`. The `serve` subcommand is
compiled behind that feature, so without it `serve` does not exist in the binary
at all — it will not appear in `--help` and `ironclaw-reborn serve` errors as an
unknown subcommand. It is an early beta operator surface, not a production
gateway. See [Running with the WebUI (`serve`)](#running-with-the-webui-serve).

## Running with the WebUI (`serve`)

`serve` starts the WebChat v2 HTTP listener so you can drive Reborn from a
browser. This is the fastest way to exercise the full loop (ingress → turn
runner → LLM provider → timeline) end to end.

**Shortcut:** `scripts/run-reborn-webui.sh` wraps the steps below — it keeps the
Reborn home outside the repo, configures the route, generates the WebUI token,
and launches `serve`. Export your provider key first, then run it:

```bash
export NEARAI_API_KEY=...                 # or OPENAI_API_KEY / ANTHROPIC_API_KEY
scripts/run-reborn-webui.sh               # NEAR AI default
PROVIDER=openai scripts/run-reborn-webui.sh
```

It prints the login token and the `http://127.0.0.1:3000/v2` URL. Override
`PROVIDER`, `MODEL`, `REBORN_HOST`, `REBORN_PORT`, or `IRONCLAW_REBORN_HOME` via
the environment. The manual steps below are equivalent.

### Quick start

```bash
# 1. For serve/run/repl the Reborn home must live OUTSIDE your current working
#    directory: these commands use the cwd as the local-dev workspace root and
#    reject overlap with it (see gotchas). Other commands have no such rule.
export IRONCLAW_REBORN_HOME="$HOME/.ironclaw-reborn-demo"

# 2. Configure a model route. NEAR AI shown here; swap the provider id and key
#    env var for any row in the table below. set-provider records the credential
#    env-var NAME in config.toml; the secret VALUE stays in the environment.
cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn -- \
  models set-provider nearai
export NEARAI_API_KEY="your-key-here"

# 3. WebUI auth. serve REQUIRES both values or it refuses to start. The variable
#    NAMES below are the defaults; override them via [webui].env_token_var and
#    [webui].env_user_id_var in config.toml if you prefer different names.
export IRONCLAW_REBORN_WEBUI_TOKEN="$(openssl rand -hex 32)"   # bearer token you log in with
export IRONCLAW_REBORN_WEBUI_USER_ID="reborn-cli"             # must match [identity].default_owner

# 4. Launch (feature flag is mandatory — the UI is not compiled in without it).
cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn -- serve
```

Then open **`http://127.0.0.1:3000/v2`** and log in with the
`IRONCLAW_REBORN_WEBUI_TOKEN` value.

`--host` / `--port` override the defaults (`127.0.0.1` / `3000`), or set
`[webui].listen_host` / `[webui].listen_port` in `config.toml`. `--port 0`
(the **CLI flag only**) tells the OS to pick a free ephemeral port — useful for
test harnesses, though the banner still prints `:0`. `[webui].listen_port = 0`
in `config.toml` is **rejected**, since a config-driven ephemeral port is almost
always a mistake. For the Slack host-beta ingress, build with
`--features slack-v2-host-beta` (it includes `webui-v2-beta`).

### Choose your model provider

`models set-provider <id>` works for any provider in the built-in catalog. For
API-key providers it records that provider's credential env-var name in
`config.toml` for you; for keyless providers (e.g. `ollama`) it writes no
`api_key_env`. The common single-API-key providers:

| Provider | `set-provider` id | Key env var | Default model |
| --- | --- | --- | --- |
| NEAR AI | `nearai` | `NEARAI_API_KEY` | `deepseek-ai/DeepSeek-V4-Flash` |
| OpenAI | `openai` | `OPENAI_API_KEY` | `gpt-5-mini` |
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` | `claude-sonnet-4-20250514` |
| Ollama (local) | `ollama` | _(none — runs locally)_ | `llama3` |

So to use Anthropic instead of the quick-start example, swap step 2 for:

```bash
cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn -- \
  models set-provider anthropic
export ANTHROPIC_API_KEY="your-key-here"
```

Not sure which env var your chosen provider needs? After `set-provider`, run
`models status` — it prints `default.api_key_env` (the exact variable to
export) alongside the active provider and model. `models list --verbose` shows
the same for every provider in the catalog, including whether its key is
`required` or `optional`; pass `--model <id>` to `set-provider` to override the
default model. Providers that use OAuth or multi-field credentials (`bedrock`,
`gemini_oauth`, `openai_codex`) need extra setup beyond a single key.

**Missing keys are fatal for required-key providers.** For `api_key_required`
providers (`openai`, `anthropic`, and most others), `run`/`serve`/`repl` exit at
startup during LLM resolution with `llm provider '<id>' requires API key env var
'<VAR>' to be set` if the env var is missing. For no-key providers (`ollama`)
and NEAR AI's session flow (`api_key_required: false`), the runtime boots
without that env var and authenticates separately — so export your provider's
key before launching `serve`.

### Common startup errors (and fixes)

These are validation failures that abort `serve` before it binds; each prints a
single-line `Error:` and exits.

| Error message contains | Cause | Fix |
| --- | --- | --- |
| `must be set to the WebChat v2 bearer token` | `IRONCLAW_REBORN_WEBUI_TOKEN` unset | Export the token env var (step 3). |
| `must be set to the UserId an env-bearer-authenticated caller maps to` | `IRONCLAW_REBORN_WEBUI_USER_ID` unset | Export the user-id env var (step 3). |
| `default_owner ... must match the WebChat v2 authenticated user` | `[identity].default_owner` ≠ `IRONCLAW_REBORN_WEBUI_USER_ID` | Set the env user to the config owner (default `reborn-cli`), or remove/align `[identity].default_owner`. |
| `workspace root must not overlap default skill root /skills` | Reborn home is **inside** the current working directory | Point `IRONCLAW_REBORN_HOME` at a path outside your repo/cwd. |

The workspace-overlap one is the easiest to trip: `serve`/`run`/`repl` use the
**current working directory** as the local-dev workspace root, and boot is
rejected if that root overlaps any default storage root Reborn manages —
`/skills` (`<reborn-home>/local-dev/skills`), `/tenant-shared/skills`,
`/system/skills`, or `/system/extensions`. If the home is nested inside the cwd
(e.g. `IRONCLAW_REBORN_HOME="$PWD/.reborn-home"`), those roots fall under the
workspace root and boot is rejected. Keep the home outside the directory you
launch from — the default `~/.ironclaw/reborn` already satisfies this.

(Resolved per-user skills live under
`<reborn-home>/local-dev/tenants/default/users/<owner>/skills`; the flat
`local-dev/skills` is a legacy root that is backfilled into that tenant-scoped
path. The validation above guards the legacy/default roots, which is why the
error names `/skills`.)

### Smoke-test a turn over the API

The browser UI talks to the `/api/webchat/v2` routes. You can drive the same
loop with `curl` to confirm the model route works without opening a browser:

```bash
TOKEN="$IRONCLAW_REBORN_WEBUI_TOKEN"
BASE=http://127.0.0.1:3000/api/webchat/v2
AUTH="Authorization: Bearer $TOKEN"

# create a thread -> returns .thread.thread_id
TID=$(curl -s -X POST "$BASE/threads" -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"client_action_id":"smoke-1"}' \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['thread']['thread_id'])")

# send a message -> queues a turn
curl -s -X POST "$BASE/threads/$TID/messages" -H "$AUTH" -H 'Content-Type: application/json' \
  -d '{"client_action_id":"smoke-msg-1","content":"Reply with exactly: NEARAI_OK"}'

# read the timeline. Turn execution is async, so re-run this until an
# assistant message with status "finalized" appears (usually a second or two).
curl -s "$BASE/threads/$TID/timeline" -H "$AUTH" | python3 -m json.tool
```

A healthy run shows a `kind: "assistant"`, `status: "finalized"` message in
`messages[]` with the model's reply (the first read right after sending may
still show only the user message — repeat the timeline request until it
finalizes). `GET /api/health` returns
`{"status":"healthy","channel":"reborn"}` and `/v2` serves the UI; `/` is
intentionally a 404. CORS is fail-closed with no allowed origins, so drive it
from a browser on the same host against `127.0.0.1`.

## Commands

### `channels list`

Reports configured Reborn channels without resolving Reborn home, reading v1 channel config, or creating directories.

The Reborn channel registry is not wired yet, so the command currently reports an explicit empty surface:

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- channels list
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- channels list --json
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- channels list --verbose
```

Expected fields include:

- `configured: 0`
- `status: not-wired`
- `v1_state: not-used`

### `extension`

Searches and manages local-dev Reborn extensions through the same lifecycle facade exposed to product surfaces. Available extension packages are read from `/system/extensions`, which maps to `<reborn-home>/local-dev/system/extensions` for the local-dev profile.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension search github
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension search github --json
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension install github-mcp
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension activate github-mcp
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension remove github-mcp
```

The commands are scoped to Reborn boot/config resolution and do not create or read v1 state directories.

Expected fields include:

- `phase`
- `package_ref.id` for package-specific commands
- `payload.kind`
- `payload.count` and `payload.extensions[].package_ref.id` for search
- `payload.installed`, `payload.activated`, or `payload.removed` for lifecycle mutations

### `completion`

Generates shell completion scripts without resolving Reborn home, reading v1 state, or creating directories.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- completion --shell zsh > ironclaw-reborn.zsh
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- completion --shell bash > ironclaw-reborn.bash
```

The zsh output keeps the v1 CLI guard around `compdef` so the generated script is safe when zsh completion functions are not loaded yet.

### `config path`

Shows the resolved Reborn state root, its source, selected profile, and explicit v1-state status without creating directories.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- config path
```

Expected fields include:

- `reborn_home`
- `home_source`
- `profile`
- `v1_state: not-used`

`config path`, `doctor`, and other read-only surfaces do not create Reborn
state or seed config files.

### `doctor`

Validates and reports Reborn boot configuration without creating state directories or starting runtime services.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- doctor
```

Expected fields include:

- `reborn_home`
- `home_source`
- `profile`
- `v1_state: not-used`
- `driver_registry: initialized`

### `hooks list`

Reports configured Reborn hooks without resolving Reborn home, reading v1 hook config, or creating directories.

The Reborn hook registry is not wired yet, so the command currently reports an explicit empty surface:

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- hooks list
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- hooks list --json
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- hooks list --verbose
```

Expected fields include:

- `configured: 0`
- `status: not-wired`
- `v1_state: not-used`

### `logs`

Reports Reborn log availability without resolving Reborn home, reading v1 gateway logs, or creating directories.

The Reborn log source is not wired yet, so the command currently reports an explicit empty surface:

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- logs
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- logs --json
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- logs --verbose
```

Expected fields include:

- `entries: 0`
- `status: not-wired`
- `v1_state: not-used`

### `onboard`

First-run bootstrap for the standalone Reborn home. It resolves
`IRONCLAW_REBORN_HOME` (or the default `~/.ironclaw/reborn`), creates the home
directory, writes missing `config.toml` and `providers.json` using the same
atomic writer as `config init`, preserves operator-edited files unless
`--force` is passed, writes a `.onboard-completed.json` marker, and prints the
remaining setup work. It does not call into v1 `src/setup`, v1 database
config, v1 channels, or v1 import state.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- onboard
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- onboard --dry-run
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- onboard --force
```

`--dry-run` reports what would be initialized without writing files.
`--import-history` reserves the history-import step in the summary (not wired
yet). See `docs/reborn/onboarding.md` for the full slice description and the
completion-marker schema.

### `models list` / `models status` / `models set-provider`

Shows Reborn model purpose slots and route status, and configures the default
LLM route.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- models list
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- models list --json
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- models status
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- models status --json
```

`models status` reports the configured default route, including the exact env
var to export for it — handy for confirming setup before `serve`/`run`:

- `default.provider`
- `default.provider_known` (`yes` once the provider id resolves in the catalog)
- `default.model`
- `default.api_key_env` (the env var that must hold your key, e.g. `NEARAI_API_KEY`)
- `default.base_url` (only when the route configures one)
- `v1_state: not-used`

Those are the **text** field names. `models status --json` serializes the
selection struct instead, nesting the route under `default` with the raw field
names: `provider_id` (not `provider`), `provider_known`, `model`, `api_key_env`,
and `base_url`.

`models list` prints the full provider catalog, marks the active provider with
`*`, and (with `--verbose`) shows each provider's `api_key_env`, default model,
and credential kind.

`models set-provider <provider> [--model <model>]` writes `[llm.default]` into
`$IRONCLAW_REBORN_HOME/config.toml` with the provider id and, for API-key
providers, its catalog credential env-var name (keyless providers like `ollama`
get no `api_key_env`). `<provider>` is a provider id or alias (`openai`,
`anthropic`, `nearai`, `ollama`, …); `--model` is optional and defaults to the
provider's catalog default.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- models set-provider openai --model gpt-5-mini
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- models set-provider nearai --model deepseek-ai/DeepSeek-V4-Flash
```

The secret value still lives in the environment under the catalog's
`api_key_env` (e.g. `OPENAI_API_KEY`, `NEARAI_API_KEY`); `set-provider` only
records the variable *name*, never the value. Once `[llm.default]` exists it
selects the provider; `LLM_BACKEND` is only an env fallback when no default slot
is configured.

### `profile list`

Lists the supported Reborn boot profiles without resolving Reborn home, reading v1 state, or creating directories.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- profile list
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- profile list --json
```

Supported profiles:

- `local-dev` (default)
- `local-dev-yolo`
- `production`
- `migration-dry-run`

Select a profile with `IRONCLAW_REBORN_PROFILE=<profile>`.

### `run`

Starts the standalone Reborn runtime and reads messages from stdin. The no-profile path targets the planned AgentLoop runtime (`reborn-planned-default`). Without model provider environment variables, the runtime still starts but messages fail cleanly because no LLM gateway is wired.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- run
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- run --message "hello"
```

Use `--dry-run` for the side-effect-free readiness snapshot:

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- run --dry-run
```

When `$IRONCLAW_REBORN_HOME/config.toml` is missing, the first stateful
runtime start through `run`, `repl`, or feature-gated `serve` seeds a sparse
`config.toml` containing `api_version` and the safe `local-dev` boot profile.
It intentionally does not seed `[llm.default]`, so env-only model selection
continues to work. `run --dry-run`, diagnostics, and read-only commands remain
side-effect-free. One-off environment selections such as
`IRONCLAW_REBORN_PROFILE=local-dev-yolo` are not persisted into the seeded
file.

Expected fields include:

- `binary: ironclaw-reborn`
- `version`
- `reborn_home`
- `home_source`
- `profile`
- `v1_state: not-used`
- `runtime_driver: planned-agent-loop`
- `driver_registry: initialized`
- `local_runtime_shell_readiness: ready`
- `planned_default_profile: available`

For `IRONCLAW_REBORN_PROFILE=local-dev-yolo`, `run`, `repl`, and `serve` require `--confirm-host-access` before the runtime receives trusted-laptop host access. Confirmed access mounts the host home through `/host`; Unix-style raw home aliases are also accepted when they can be represented as scoped mount aliases.

When `serve --confirm-host-access` grants trusted-laptop access, `serve` refuses non-loopback listeners such as `0.0.0.0`. Bind to `127.0.0.1` or `::1`, or use a less privileged profile for non-loopback test listeners.

For `IRONCLAW_REBORN_PROFILE=production`, `run` requires production storage
and an explicit runtime policy:

```toml
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
# Optional; defaults to 2. Keep below the PostgreSQL server or managed
# session-pool cap after reserving capacity for restarts and operator sessions.
pool_max_size = 2

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
```

Set `IRONCLAW_REBORN_POSTGRES_URL` in the process environment, and set
`IRONCLAW_REBORN_SECRET_MASTER_KEY` to independent cryptographic key material.
Remote managed PostgreSQL URLs must use TLS, for example `sslmode=require`.
Set `IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE` to override the configured pool
size when a managed provider enforces a smaller session-pool cap.
The first production launch slice supports runtime policies that do not require
a tenant-sandbox process binding.

### `repl`

Starts an interactive Reborn session backed by the composed runtime, reading
turns from stdin. Same runtime as `run`, without the WebUI listener. Accepts
`--confirm-host-access` for `local-dev-yolo`.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- repl
```

### `serve`

Starts the WebChat v2 HTTP listener (browser UI). Requires a binary built with
`--features webui-v2-beta` and the two `IRONCLAW_REBORN_WEBUI_*` auth env vars.
See [Running with the WebUI (`serve`)](#running-with-the-webui-serve) for the
full walkthrough, auth setup, common startup errors, and an API smoke test.

```bash
cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn -- serve
cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn -- serve --host 127.0.0.1 --port 3000
```

### `skills list`

Reports configured Reborn local-dev skills from `<reborn-home>/local-dev/skills`
and `<reborn-home>/local-dev/system/skills` through the Reborn composition
skill listing function. It does not read v1 skill discovery paths, and a missing
local-dev storage root is reported as an empty skill list without creating
directories.

```bash
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- skills list
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- skills list --json
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- skills list --verbose
```

Expected fields include:

- `configured: <count>`
- `source: reborn-local-dev`
- per-skill `name`, `source`, and `description` in text output
- per-skill `name`, `version`, `description`, `source`, `keywords`, `tags`,
  and `requires_skills` in JSON output

`--verbose` adds the resolved `profile`, `reborn_home`, `local_dev_root`, and
`owner_id`; text output also includes per-skill `version`, `keywords`, `tags`,
and `requires_skills` when present. `skills list` currently supports
`local-dev` and `local-dev-yolo` profiles and rejects `production` /
`migration-dry-run` until those catalog backends are wired.

## State and config root

Reborn must not use the current v1 IronClaw state root by default.

Home resolution precedence:

1. `IRONCLAW_REBORN_HOME`
2. `~/.ironclaw/reborn`

The resolver rejects unsafe or misleading homes, including empty paths, relative paths, filesystem root, parent-directory components, and known v1 state-root aliases such as `$HOME/.ironclaw` or `IRONCLAW_BASE_DIR`.

## Profiles

Use `IRONCLAW_REBORN_PROFILE` to select the boot profile.

Supported values:

- `local-dev` (default)
- `local-dev-yolo`
- `production`
- `migration-dry-run`

Example:

```bash
IRONCLAW_REBORN_HOME="$PWD/.reborn-home" \
IRONCLAW_REBORN_PROFILE=production \
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- doctor
```

## Local smoke checks

Run these before changing Reborn CLI behavior:

```bash
cargo fmt --all -- --check
cargo test -p ironclaw_reborn_cli
cargo test -p ironclaw_reborn_config
cargo test -p ironclaw_reborn model_slots_are_exposed_in_cli_display_order
cargo test -p ironclaw_architecture reborn
cargo clippy -p ironclaw_reborn_cli --all-targets -- -D warnings
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- --help
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- channels list
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- completion --shell zsh >/tmp/ironclaw-reborn.zsh
IRONCLAW_REBORN_HOME="$(mktemp -d)/reborn-home" \
  cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- config path
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- hooks list
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- logs
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- models status
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- profile list
IRONCLAW_REBORN_HOME="$(mktemp -d)/reborn-home" \
  cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- run
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- skills list
```

## Adding commands

Future commands should follow the crate-local agent contract in:

```text
crates/ironclaw_reborn_cli/AGENTS.md
```

Short version:

1. add one command module under `crates/ironclaw_reborn_cli/src/commands/`;
2. register it in `commands::Command`;
3. resolve and pass `RebornCliContext` from dispatch only when the command needs boot config;
4. keep pure commands independent from Reborn home resolution;
5. add a binary smoke test through `env!("CARGO_BIN_EXE_ironclaw-reborn")`;
6. avoid v1 runtime imports and v1 state mutation unless explicitly scoped and guarded.

Do not port the current `src/cli/*` command tree wholesale. Port commands one at a time, starting with Reborn-owned or read-only surfaces.

## Release packaging decision

`ironclaw-reborn` is **not yet included in cargo-dist release artifacts**.

Current `dist plan --output-format=json` with `crates/ironclaw_reborn_cli` marked `dist = false` emits only the root `ironclaw` package artifacts. Removing `dist = false` alone is not enough to ship `ironclaw-reborn` in the existing `ironclaw-v*` release workflow because that workflow is shaped around the root `ironclaw` package tag. Enabling a standalone `ironclaw_reborn_cli` release also requires cargo-dist WiX metadata/template work and an explicit tag/versioning decision.

Follow-up issue: #3483 tracks packaging `ironclaw-reborn` in release artifacts.

Until #3483 is resolved, keep:

```toml
[package.metadata.dist]
dist = false
```

in `crates/ironclaw_reborn_cli/Cargo.toml` so releases do not silently claim to ship an unverified Reborn binary package.
