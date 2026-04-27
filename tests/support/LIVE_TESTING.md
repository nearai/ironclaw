# Live Testing Guide

`tests/e2e_live*.rs` are scenarios that exercise the real agent against
real LLMs and real tools, then capture the run as a JSON trace fixture
under `tests/fixtures/llm_traces/live/`. The fixture lets the same test
re-run deterministically in CI in *replay* mode without ever calling out
to a paid LLM.

For scheduled live CI lanes, runner setup, and release gating policy, see
`docs/internal/live-canary.md`.

## Modes

`LiveTestHarnessBuilder::build()` picks one of two modes based on the
`IRONCLAW_LIVE_TEST` environment variable.

| Mode | When | Behavior |
|------|------|----------|
| **Live** | `IRONCLAW_LIVE_TEST=1` | Resolves the real config from `~/.ironclaw/.env`, builds the real LLM provider chain, runs the scenario, writes a fresh trace fixture to `tests/fixtures/llm_traces/live/<test_name>.json`. |
| **Replay** (default) | unset / `0` / empty | Loads the existing trace fixture and replays it through `TraceLlm`. No network calls. Used in CI. |

Run a single live test:

```bash
# Live recording (writes a new trace + .log)
IRONCLAW_LIVE_TEST=1 cargo test --test e2e_live -- zizmor_scan --ignored --nocapture

# Replay only (deterministic)
cargo test --test e2e_live -- zizmor_scan --ignored
```

## Database Contract — Read This Before Adding a Live Test

The test rig **always starts with a fresh, empty libSQL database**. It
does not clone the developer's `~/.ironclaw/ironclaw.db`. This is
deliberate:

- **No accidental PII**: workspace memory, conversation history, and
  per-user secrets stay in the source DB. They never leak into the
  test rig, the recorded trace, or any committed fixture.
- **No latent dependencies**: tests cannot pass "by accident" because
  some unrelated row happened to be in the developer's local DB. If a
  test needs state to exist, it has to put it there.

If your test needs **specific** credentials from the real DB (e.g. an
OAuth token already configured for Gmail), declare them by name:

```rust
let harness = LiveTestHarnessBuilder::new("gmail_send_test")
    .with_engine_v2(true)
    .with_secrets(["google_oauth_token"]) // ← only these rows are copied
    .build()
    .await;
```

`with_secrets` opens `~/.ironclaw/ironclaw.db` read-only, copies the
listed rows from the `secrets` table under your `owner_user_id`, and
re-inserts them into the temp DB under the same owner. Any name not
present in the source is logged as a warning and the test will fail
fast on its own missing-credential path.

If your test needs **memory or workspace state**, seed it explicitly
through the rig's APIs (e.g. `rig.workspace().write(...)`) inside the
test body — do not assume the developer's local DB has it.

## PII Scrub Checklist — Before Committing a Recorded Trace

Live runs write two files into `tests/fixtures/llm_traces/live/`:

| File | What it contains |
|------|------------------|
| `<test_name>.json` | LLM exchanges, tool inputs/outputs, final responses |
| `<test_name>.log`  | Tracing log lines from the agent during the run |

Both files are **committed to the repo**. The contract is that **the
test author scrubs personal data before committing**. The harness
keeps the surface area small (no full DB clone, secrets only by
explicit opt-in) but it cannot know what the real tools wrote into a
prompt or response.

Before `git add tests/fixtures/llm_traces/live/<test_name>.{json,log}`,
do a focused review:

1. **Search for credentials and tokens**:
   ```bash
   grep -niE 'bearer |api[_-]?key|secret|password|access_token|refresh_token' \
     tests/fixtures/llm_traces/live/<test_name>.{json,log}
   ```
2. **Search for PII**: real names, email addresses, phone numbers,
   street addresses, billing identifiers, account numbers.
3. **Search for private URLs / hostnames** that aren't intended to be
   public (internal Slack channels, ngrok tunnels, internal Linear
   workspaces, etc).
4. **Search for the encrypted secret payloads**: even though the
   `secrets` table is encrypted at rest, a tool may have *used* a
   credential and the cleartext could appear in an HTTP request body
   or a response excerpt.
   ```bash
   grep -ciE '[a-z0-9_]{32,}' tests/fixtures/llm_traces/live/<test_name>.json
   ```
   Any high-entropy looking string deserves a second look.
5. **Inspect tool result previews**: trace files truncate tool outputs
   at ~200 chars, but that is plenty of room for an OAuth token or a
   real email address. Open the file and skim every `tool_result`
   block.

If you find anything sensitive, either:
- redact it inline (replace with `<REDACTED>` and keep the structure
  recognizable to the replay harness), **or**
- delete the offending step and re-record under a more sanitized
  scenario, **or**
- leave the trace uncommitted and reproduce the run on demand instead.

When in doubt: **do not commit**. A live test that has to be
re-recorded by each developer is annoying; a leaked credential is a
revocation incident.

## Adding a New Live Test

1. Add a `#[tokio::test]` + `#[ignore]` (live tier — never runs in the
   default `cargo test` matrix) in `tests/e2e_live*.rs`.
2. Build the harness with `LiveTestHarnessBuilder::new("<unique_name>")`.
3. If you need credentials, list them via `.with_secrets([...])`.
4. Drive the rig (`rig.send_message`, `rig.wait_for_responses`, …).
5. Record the trace once with `IRONCLAW_LIVE_TEST=1`.
6. Run the PII scrub checklist above.
7. Commit the test code AND the scrubbed trace.

## Why not just `.gitignore` the trace files?

The replay fixtures are a feature, not a leak: they let CI run live
scenarios deterministically without paid LLM calls. Gitignoring them
would force every contributor to re-record before each run, which is
both expensive and *more* prone to PII leakage (each developer's local
DB is different). The cleaner contract is "fixtures are committed,
authors scrub them, the rig narrows the surface area".
