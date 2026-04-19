# Fork Context

This repository is a hard fork of IronClaw. Future agents should treat this
file as fork-level context, not as an upstream compatibility promise.

## Fork Goal

The fork prioritizes:

- XMPP/OMEMO-first agent operation.
- Stable encrypted group chat behavior.
- Reliable scheduled routines.
- Local/self-hosted deployment.
- Practical systemd operations.
- Clear operational scripts over hidden manual steps.

Compatibility with upstream IronClaw is not required unless a task explicitly
requires it.
Please also see README.md and MANIFESTO for additional information about what the purpose of this hard fork is and what kinds of features are prioritized. Do not edit README.md or MANIFESTO unless explicitly asked.

## Upstream Policy

- Do not assume upstream compatibility.
- Do not preserve code paths only for upstream parity.
- Prefer clear fork-local behavior over generic abstractions when they conflict.
- Upstream merges, if any, should be treated as deliberate porting work, not
  routine synchronization.
- If a change modifies behavior tracked in `FEATURE_PARITY.md`, update that
  file only when the fork still cares about that parity entry.

## Protected Runtime Behavior

Do not break these without explicit approval:

- XMPP bridge operation.
- XMPP room configuration through the gateway.
- OMEMO encrypted one-to-one and group chat support.
- XMPP group chat self-message suppression.
- XMPP live rate-limit control.
- WASM channel loading.
- WASM tool loading.
- Gotify WASM tool usage.
- Scheduled routines.
- Routine manual runs.
- Gateway status/config endpoints.
- Systemd deployment units.
- IronClaw watchdog service/timer.

## Runtime Shape

Expected runtime pieces may include:

- Main IronClaw daemon service.
- Separate XMPP bridge service.
- WASM channels, especially XMPP.
- WASM tools, especially Gotify.
- Gateway API used for status and runtime configuration.
- Systemd units and helper scripts under `systemd/` and `scripts/`.

The XMPP bridge is a separate process that connects to XMPP as a normal user,
not as a Prosody component. OMEMO encryption/decryption happens in the XMPP
bridge/client path, not in the main daemon.

## Deployment Rules

- Codex may build binaries when asked.
- Codex must not deploy binaries unless explicitly asked.
- Codex must not restart services unless explicitly asked.
- For XMPP bridge changes, build and verify first; deployment is a separate
  step unless requested.
- When touching systemd units, call out whether daemon restart, bridge restart,
  or both are required.

## Secrets

Never commit secrets.

Secrets may exist in:

- `/home/cmc/.ironclaw/.env`
- systemd service environment.
- local database rows.
- local WASM tool/channel auth state.

Agents may inspect whether required keys exist, but must not print secret
values in logs, diffs, or final answers.

## Build and Test

Preferred baseline commands:

```sh
cargo test
cargo build --release
```

Use targeted checks when possible. For XMPP bridge work, build the bridge
binary specifically before doing a wider workspace build.

For live database checks, use read-only SQL unless the user explicitly requests
repair or mutation.

## Database and Routine State

Scheduled routines can become blocked if old rows in `routine_runs` remain in
`status='running'`. Most routines use `max_concurrent=1`, so one stale running
row can block future scheduled runs.

Known failure mode:

- Lightweight routine runs can be stuck with `job_id IS NULL`.
- The current recovery path may not finalize those rows.
- Full-job routine runs with old pending jobs may also remain active until the
  linked job state is resolved.

Operational cleanup should be explicit and careful:

- Stop the service before mutating routine state.
- Back up the database first.
- Mark stale routine runs failed or cancelled.
- Restart and verify that `running` routine counts are clear.

## Gotify Notes

Gotify is currently treated as a WASM tool, not an IronClaw channel.

Do not set a routine's notification channel to `gotify` unless a separate
Gotify channel implementation is registered. A routine that needs Gotify should
call the `gotify` tool inside its prompt and return the same message as backup
output.

See `docs/GOTIFY_ROUTINE_PROMPT.md` for the current working prompt pattern.

## XMPP and OMEMO Notes

Known operational behavior:

- The XMPP bridge rejects conflicting runtime config with HTTP 409 until the
  bridge is restarted.
- Bridge room membership should be checked through the gateway status endpoint.
- OMEMO encrypted group chat can take several messages after restart before
  decrypting reliably.
- Group chat OMEMO requires the room to be configured/enabled as encrypted in
  both the client setup and the bridge/runtime configuration.
- The agent nickname/resource may come from XMPP service environment and bridge
  configuration, so verify both when changing display identity.

## Repository Boundaries

Canonical source areas:

- `src/`
- `bridges/`
- `channels-src/`
- `tools-src/`
- `migrations/`
- `docs/`
- `systemd/`
- `scripts/`

Generated, disposable, or review-carefully areas:

- `target*`
- copied working trees.
- temporary experiment folders.
- old backup directories.

Do not assume every directory in this working tree is canonical. When unsure,
ask or verify with git status and recent task context.

## Change Discipline

- Keep changes scoped.
- Respect dirty worktrees.
- Do not revert user changes unless explicitly asked.
- Avoid broad refactors unless the task requires them.
- Preserve security-sensitive behavior around auth, secrets, network listeners,
  approvals, CORS, rate limits, and sandboxing.
- Document behavior changes in the relevant docs/specs when the change is
  user-visible.

