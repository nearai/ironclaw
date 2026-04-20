# Live-canary runner (Railway)

Self-hosted GitHub Actions runner for the single canary lane that exercises
real hosted OAuth roundtrips against live provider endpoints
(`.github/workflows/live-canary.yml` → `private-oauth`). See the comment on
that job, and the "Why private-oauth specifically" section in the review
thread, for the reasoning — the short version is:

- The lane runs a real code-for-token grant + refresh against Google.
- The provider OAuth app has a fixed redirect URI bound to this runner's
  egress IP / hostname.
- Rotated refresh tokens are written to ironclaw's libsql DB and must
  survive container restarts.

None of that works on rotating GitHub-hosted runner IPs or ephemeral
containers without state.

## What's in this directory

| File | Role |
|------|------|
| `Dockerfile` | Ubuntu 22.04 base + `gh`, `git`, `build-essential`. Rust is installed per-job by the workflow; its cache persists via `CARGO_HOME` / `RUSTUP_HOME` on the volume. |
| `entrypoint.sh` | First-boot: downloads the runner, registers with `GH_RUNNER_TOKEN`. Subsequent boots: `exec ./run.sh`. |

## One-time bring-up

Order matters — the runner has to exist and be running before anything
actually exercises it.

### 1. Railway project + service + volume

- Create a Railway project, then a service sourced from this directory
  (`infra/runner/`). Railway will build the `Dockerfile` on push.
- Attach a **persistent volume** to the service, mounted at
  `/runner-data`. Size ~20 GB — covers runner self-updates, cargo/rustup
  caches, ironclaw `target/`, and the libsql DB.
- Set the deploy strategy to **"Overlap: off" / Recreate** (not rolling):
  rolling deploys could kill a container mid-OAuth-refresh, leaving the
  refresh token rotated on the provider side but not persisted locally.

### 2. Reserve a static egress IP

Enable Railway's static outbound IP on the service (Pro/Team plan
feature). Write down the IP — you'll register it in step 4.

### 3. Register the runner against GitHub

- GitHub: `Settings → Actions → Runners → New self-hosted runner` →
  Linux → copy the **registration token**. The token is valid for ~1h.
- Railway env on the service:
  - `GH_RUNNER_URL` = `https://github.com/<ORG>/<REPO>`
  - `GH_RUNNER_TOKEN` = the token from the step above (one-shot)
  - `RUNNER_NAME` = e.g. `railway-private-oauth` (optional, defaults to
    that)
  - `RUNNER_LABELS` = `self-hosted,ironclaw-live` (optional, defaults to
    that — must include both for the workflow's `runs-on` match)
- Deploy. The container boots, `entrypoint.sh` downloads the runner,
  `./config.sh` registers it, and `./run.sh` starts polling.
- Confirm the runner shows up as "Idle" at
  `Settings → Actions → Runners`.
- **Delete `GH_RUNNER_TOKEN` from Railway env** — it's spent, and keeping
  expired secrets around is noise.

### 4. Register the egress IP with the provider OAuth app(s)

- Google Cloud Console → Credentials → the OAuth 2.0 Client ID the
  canary uses → add an authorized redirect URI with the runner's public
  hostname (if Railway gave you one) or the static egress IP.
- Same for any other provider the lane touches in the future.

### 5. Canary secrets go on the runner, not on GitHub

Unlike the other live lanes, `private-oauth` does **not** declare any
`env:` entries exposing `GOOGLE_OAUTH_CLIENT_ID` / `_SECRET` on the
job. The whole point of the `dedicated-runner` pattern is that the
runner has its own identity — the test process inherits these from the
runner's own env, so they live in **Railway service env**, not GitHub
Actions secrets:

- `GOOGLE_OAUTH_CLIENT_ID` — the client_id of the Google OAuth app
  whose redirect URI you registered in step 4.
- `GOOGLE_OAUTH_CLIENT_SECRET` — the matching client_secret.
- Any other provider creds the lane grows to cover (Notion, etc.)
  follow the same pattern.

The runner binary runs as the Railway container process, so these vars
are visible to `actions/runner/run.sh` → `Runner.Listener` →
`Runner.Worker` → the cargo test process, in that order. No other
workflow or repo has access to them.

### 6. Verify

Trigger the lane ad-hoc:

```bash
gh workflow run live-canary.yml \
  --ref main \
  -f lane=private-oauth
```

Watch `Actions → Live Canary → Private OAuth Live` — the job should
pick up on the `railway-private-oauth` runner. First run takes ~8–10
minutes (full cargo build + cargo-component install). Subsequent runs
should drop to ~2–3 minutes once the volume caches warm.

## Operations

### Updating the runner version

GitHub releases a new `actions/runner` every ~2 weeks. The runner
auto-updates in place on the volume, so most updates need no action.
When a major release bumps the minimum supported version, rebuild the
image with a fresh `--build-arg RUNNER_VERSION=<new>` so first-boot
works on a wiped volume.

### Rotating the Google OAuth client secret

1. Generate a new client secret in Google Cloud Console; leave the old
   one active.
2. Update `GOOGLE_OAUTH_CLIENT_SECRET` in GitHub repo secrets.
3. Trigger the lane; confirm it passes on the new secret.
4. Revoke the old secret in Google Cloud Console.

The refresh token on the Railway volume is bound to the client, not to
the specific secret, so rotation is non-disruptive.

### Recovering from a stuck refresh token

If the libsql DB holds a refresh token Google has already revoked
(happens if the DB wasn't on the volume during a deploy, or if the
provider invalidated the session), the lane fails with a token-refresh
error. Recovery:

1. Trigger the `drive_auth_gate_roundtrip` flow manually against the
   runner (via the normal ironclaw onboarding UI, pointed at the
   runner's gateway).
2. That re-mints a fresh refresh token and writes it to the volume DB.
3. Re-run `private-oauth`.

### Rebuilding from scratch

`railway volume wipe` (or delete + recreate the volume) clears all
state. After wipe:
- Regenerate `GH_RUNNER_TOKEN` in GitHub and set it in Railway env.
- Redeploy. First boot re-registers the runner using the same
  `RUNNER_NAME`, which collides with the old offline registration;
  `--replace` in the config call handles that automatically.

## What NOT to put here

- Rust toolchain installations in the `Dockerfile`. The workflow picks
  its exact toolchain via `dtolnay/rust-toolchain`; duplicating in the
  image causes version drift.
- A second runner instance. The lane runs hourly and tolerates the
  ~minutes of downtime during a Railway deploy. If that changes, add
  a sibling service with `RUNNER_NAME=railway-private-oauth-2` and the
  same labels; GitHub round-robins across matching runners.

## What goes where (secrets layout)

| Secret | Location | Why |
|--------|----------|-----|
| `GH_RUNNER_TOKEN` | Railway env (then deleted after first boot) | One-shot registration token. Expires in ~1h. |
| `GOOGLE_OAUTH_CLIENT_ID` / `_SECRET` | **Railway env** | The lane's `private-oauth` job deliberately doesn't expose these via `env:` — the runner is the identity. |
| Rotated OAuth refresh tokens | Runner volume (`/runner-data/home/.ironclaw/…libsql db`) | Must survive container restart. Encrypted at rest by Railway. |
| Any `AUTH_LIVE_*` tokens | GitHub Actions secrets | Used by `auth-live-seeded` (a different lane, on `ubuntu-latest`). Not this lane. |
