# IdentyClaw Passport add-on plan

Status: proposed upstream (nearai/ironclaw, branch `feat/identyclaw-passport`, 2026-08-11).
Audience: practitioners adding Passport to IronClaw agents that do not ship it
(e.g. stock Nous / nearai builds), and maintainers packaging that path.

## Problem

Nous-shipped IronClaw agents typically lack IdentyClaw Passport. This fork
already has the full stack (`builtin.idcp`, host helper, skill, Podman wiring),
but that is not how most practitioners run IronClaw, and Passport must not be
framed as “install like Slack.”

## Non-goals

- Do **not** package Passport as a normal installable extension (WASM / Settings
  catalog) that holds NEAR keys or JWTs in extension runtime.
- Do **not** require Podman, a compose sidecar, or this repo’s full deploy kit.
- Do **not** teach the model to curl a helper URL or paste JWTs/keys.
- Do **not** add a Cargo feature solely to toggle Passport — the hard part is
  host credential custody and ops, not a second workspace build.

## Hard invariant

Passport private keys and full JWTs stay **host-side**. The agent only sees a
mediated, redacted surface (`builtin.idcp` and/or shell `idcp` verbs steered by
the skill).

## What is actually required

| Need | Required? | Notes |
| --- | --- | --- |
| Host credential keeper (NEAR key + JWT cache on disk) | **Yes** | Today: `deploy/identyclaw` Node lib + CLI |
| Loopback HTTP helper process (`server.mjs` on `:3921`) | **Only for `builtin.idcp`** | Not needed if the agent uses shell `idcp` |
| Podman / container sidecar | **No** | One optional packaging of the helper |
| Runtime skill `skills/identyclaw` | **Yes** (UX) | Steers the model; alone is insufficient |
| `builtin.idcp` in the agent binary | **Yes for processless profiles** | Needed when `builtin.shell` is hidden |
| Shell `idcp` on PATH | Optional fallback | Works only when process tools are visible |

A “sidecar” in docs means “host process that holds keys,” not “must be a
container next to the agent.”

## Target packaging: two artifacts

### 1. Thin host seam (binary / upstream)

Keep (or upstream into Nous IronClaw) only:

- `builtin.idcp` first-party capability (loopback HTTP → helper, redaction)
- Policy grant + AskAlways exemption for `builtin.idcp`

No Rodit/NEAR crypto in the agent image. No Passport product logic beyond
“call helper, redact.”

**Stock Nous gap:** without this seam, processless agents cannot use Passport
even if practitioners install the add-on kit.

### 2. Practitioner add-on kit (what people install)

Ship as a **standalone, versioned host package** — npm-first, container optional.

Suggested layout (extract/refine from `deploy/identyclaw/`):

```text
ironclaw-identyclaw-addon/   # or publish name: idcp / @identyclaw/ironclaw-addon
  package.json               # bin: idcp
  src/cli.mjs                # enroll, ensure_session, me, HOLA, request, …
  src/server.mjs             # optional loopback HTTP for builtin.idcp
  src/lib.mjs
  vendor/hola-client/
  skill/                     # copy of skills/identyclaw/SKILL.md
  README.md                  # practitioner recipe (no Podman assumed)
```

Optional later: OCI image wrapping the same helper for people who already
containerize; keep it secondary in the README.

## Practitioner recipes (default = no Podman)

### A. Shell-enabled local agent (simplest)

```bash
# 1) Install host tool
cd deploy/identyclaw && npm install --omit=dev
# or: npm i -g <published-addon>

# 2) Enroll + mint Passport
idcp enroll
# Human: https://purchase.identyclaw.com with printed account_id

# 3) Session check
idcp ensure_session && idcp me

# 4) Skill
cp -R skills/identyclaw ~/.ironclaw/skills/identyclaw
# or workspace skills/identyclaw/

# 5) Ensure idcp is on PATH for the agent process (shell path)
```

Agent uses skill → `builtin.shell` → `idcp …`. No HTTP helper required.

### B. Processless / `builtin.idcp` (volume / secure-default)

Same as A, plus:

```bash
# Beside ironclaw serve (same host network namespace / localhost)
IDENTYCLAW_HELPER_HOST=127.0.0.1 IDENTYCLAW_HELPER_PORT=3921 npm start
# Agent env:
export IDENTYCLAW_HELPER_BASE=http://127.0.0.1:3921
```

Requires an agent binary that already exposes `builtin.idcp`.

### C. Podman deploy kit (optional)

Keep current `./ironclaw.sh idcp-*` + compose sidecar as a convenience path for
operators already on `deploy/podman/`. Do not present it as the primary add-on.

## What not to build first

| Idea | Why defer |
| --- | --- |
| Settings-installable `identyclaw` extension with tools | Wrong trust boundary for keys; fights “host-mediated only” |
| Skill-only drop-in for stock Nous processless | No capability / no shell → agent cannot complete login |
| MCP wrapper of the helper | Viable later for Settings UX; still needs the host keeper; policy story differs from `DispatchCapability`-only `builtin.idcp` |
| In-process Rodit crypto inside `ironclaw` | Larger binary + secret custody redesign; revisit only if the Node helper is a real ops burden |

## Workstreams (when we act)

### W1 — Document and extract the add-on

- [ ] Rewrite practitioner docs so Podman is “optional deploy,” not the default
- [ ] Produce a standalone README recipe (A/B above) at add-on root
- [ ] Decide publish channel: npm package vs git subdirectory consumers clone
- [ ] Pin Node engine and `@rodit/rodit-auth-be` resolution for out-of-tree installs
- [ ] Bundle `skills/identyclaw` in the add-on tarball/package

### W2 — Host seam for stock Nous

- [ ] Inventory whether upstream Nous binary has `builtin.idcp` (expect: no)
- [ ] Thin PR / patch: capability + policy grant/exemption only
- [ ] Fail closed with a clear `identyclaw_helper_unreachable` hint when helper absent
- [ ] Do not block stock builds on helper presence at boot

### W3 — DX polish

- [ ] `idcp doctor` — checks creds dir, session, loopback helper reachability
- [ ] One-shot `idcp setup` that prints next human step (purchase URL + skill path)
- [ ] Soften error copy in `builtin.idcp` to mention `npm start` / `idcp`, not only “sidecar”

### W4 — Optional later

- [ ] Hosted-MCP façade over the same helper for Settings-install UX
- [ ] Compose/Docker fragment as an *optional* appendix (not the headline)
- [ ] IronHub / skill-registry distribution of `identyclaw` skill only (still needs W1+W2)

## Acceptance criteria

1. A practitioner on a laptop with Node 20+, without Podman, can enroll a Passport
   and have an IronClaw agent complete `ensure_session` / `me` / HOLA without
   seeing keys or JWTs.
2. Processless profile works when binary has `builtin.idcp` + local `npm start`
   helper; shell profile works with CLI alone.
3. Primary docs never require Podman.
4. No new installable extension that stores Passport material in the agent sandbox.

## Current in-tree anchors

| Piece | Path |
| --- | --- |
| Capability | `crates/kernel/ironclaw_host_runtime/src/first_party_tools/idcp.rs` |
| Policy | `crates/app/ironclaw_composition/src/builtin_capability_policy.toml` |
| Helper + CLI | `deploy/identyclaw/` |
| Skill | `skills/identyclaw/SKILL.md` |
| Podman convenience | `ironclaw.sh` (`idcp` / `idcp-init`), `deploy/podman/README.md` |
| Product blurb | `README.md` § IdentyClaw Passport, `docs/internal/identyclaw-passport.md` |

## Decision log

| Date | Decision |
| --- | --- |
| 2026-08-10 | Passport is host-mediated, not an installable extension |
| 2026-08-10 | Container sidecar is optional packaging; host Node helper + skill is the add-on |
| 2026-08-10 | Podman is not the default practitioner path |
| 2026-08-10 | `builtin.idcp` remains the processless seam; shell `idcp` is the simple fallback |
