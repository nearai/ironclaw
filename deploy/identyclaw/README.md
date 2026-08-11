# Host-side IdentyClaw helper

Passport keys stay on the host. JWTs are cached on disk. The agent never sees
private keys or full JWTs.

**Agent interface:** prefer **`builtin.idcp`** (processless-safe). On
shell-enabled profiles, `idcp` on PATH is an optional fallback with the same
verbs. Do not teach the model to curl the loopback helper.

Longer packaging notes: [`docs/internal/identyclaw-passport.md`](../../docs/internal/identyclaw-passport.md)
and [`docs/internal/plans/2026-08-10-identyclaw-passport-addon.md`](../../docs/internal/plans/2026-08-10-identyclaw-passport-addon.md).

## Practitioner setup (no containers required)

```bash
cd deploy/identyclaw
npm install --omit=dev
./bin/idcp enroll                 # NEAR key → credentials dir
# Human: https://purchase.identyclaw.com with account_id
./bin/idcp ensure_session
./bin/idcp me
./bin/idcp create_hola --recipient MUNDO
```

Put `deploy/identyclaw/bin` on `PATH`, or symlink `idcp`. Copy
`skills/identyclaw` into the agent skills directory.

### Processless path (`builtin.idcp`)

```bash
# Beside ironclaw serve (same host network namespace / localhost)
IDENTYCLAW_HELPER_HOST=127.0.0.1 IDENTYCLAW_HELPER_PORT=3921 npm start
export IDENTYCLAW_HELPER_BASE=http://127.0.0.1:3921
```

```text
WebUI / agent turn
  → CapabilityHost invoke(builtin.idcp)
  → host-runtime idcp handler
  → HTTP to IDENTYCLAW_HELPER_BASE (loopback only)
  → helper (NEAR key + JWT on disk) → api.identyclaw.com
  → redacted JSON back to model
```

| Piece | Choice |
|-------|--------|
| Capability id | `builtin.idcp` |
| Ops | `ensure_session`, `me`, `request`, `create_hola`, `verify_hola`, `agents`, `info`, `list_sessions` |
| Effects | `DispatchCapability` only |
| Helper base | Host env; must be `http://127.0.0.1` / `localhost` / `::1` |
| Redaction | JWT-shaped strings + sensitive key names stripped |
| Enroll | Host-only: `idcp enroll` |

`identyclaw` / `identyclaw-init` remain aliases of `idcp` / `idcp-init` where
wrappers exist.

## Optional container packaging

`Containerfile` builds the same helper as an OCI image for operators who already
containerize. It is optional — not the default practitioner path.
