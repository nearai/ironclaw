# IdentyClaw Passport on IronClaw

Practitioner guide for using [IdentyClaw](https://api.identyclaw.com) Passport
with stock IronClaw. Passport private keys and full JWTs stay **host-side**;
the agent only sees a mediated, redacted surface.

## What ships in this tree

| Piece | Path | Role |
| --- | --- | --- |
| `builtin.idcp` | `crates/kernel/ironclaw_host_runtime/src/first_party_tools/idcp.rs` | First-party capability (loopback HTTP → helper, redaction) |
| Policy grant + AskAlways exemption | `crates/app/ironclaw_composition/src/builtin_capability_policy.toml` | Visible on processless profiles; no per-call approval stall |
| Host helper + `idcp` CLI | `deploy/identyclaw/` | NEAR key + JWT cache; optional loopback HTTP on `:3921` |
| Runtime skill | `skills/identyclaw/SKILL.md` | Steers the model to prefer `builtin.idcp` |
| Packaging notes | `docs/internal/plans/2026-08-10-identyclaw-passport-addon.md` | Non-goals, recipes, future add-on extraction |

Passport is **not** an installable Settings extension. Do not put NEAR keys or
JWTs in WASM / MCP extension runtime.

## Practitioner recipe (default = no containers)

Requires Node 20+ on the host beside `ironclaw`.

```bash
# 1) Install host tool
cd deploy/identyclaw && npm install --omit=dev

# 2) Enroll + mint Passport
./bin/idcp enroll
# Human: https://purchase.identyclaw.com with the printed account_id

# 3) Session check
./bin/idcp ensure_session && ./bin/idcp me

# 4) Skill (workspace or user skills dir)
cp -R skills/identyclaw ~/.ironclaw/skills/identyclaw

# 5a) Shell-enabled agent: put idcp on PATH for the agent process
# 5b) Processless / builtin.idcp: start the loopback helper beside serve
IDENTYCLAW_HELPER_HOST=127.0.0.1 IDENTYCLAW_HELPER_PORT=3921 npm start
# Agent env:
export IDENTYCLAW_HELPER_BASE=http://127.0.0.1:3921
```

```text
Agent turn → builtin.idcp → http://127.0.0.1:3921 (helper) → api.identyclaw.com
```

Supported ops: `ensure_session`, `me`, `request`, `create_hola`, `verify_hola`,
`agents`, `info`, `list_sessions`. Enrollment stays host-only (`idcp enroll`).

If the helper is absent, `builtin.idcp` returns model-visible
`identyclaw_helper_unreachable` and does **not** fail the run. Stock builds do
not require the helper at boot.

## Trust boundary

- Prefer `builtin.idcp` over hand-rolled `/api/login` or pasting JWTs.
- Helper base must be literal loopback (`127.0.0.1` / `localhost` / `::1`).
- Responses are redacted (JWT-shaped strings + sensitive key names).
- Under AskAlways, `builtin.idcp` is approval-gate exempt because secrets never
  reach the model; keys remain on the host helper either way.

See also: [`deploy/identyclaw/README.md`](../../deploy/identyclaw/README.md),
[`skills/identyclaw/SKILL.md`](../../skills/identyclaw/SKILL.md).
