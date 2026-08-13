# @rodit/hola-client

Node helpers for IdentyClaw **HOLA line** construction — uppercase canonicalization, RFC 4648 base32 signatures, and mod-23 checksum.

This package handles **HOLA protocol signing only**. It does **not** perform API login. You need a separate **API bearer token** (`jwt_token` from `POST /api/login`) before calling `getNonce` or `createHola`.

| Concern | This package | IdentyClaw server docs |
| --- | --- | --- |
| API session (JWT) | Caller supplies `jwt` | [login-authentication.md](https://github.com/discernible-io/idclawserver-idc/blob/main/references/login-authentication.md) |
| HOLA nonce | `getNonce` → `GET /api/holanonce16ts` | [holanonce-api.md](https://github.com/discernible-io/idclawserver-idc/blob/main/references/holanonce-api.md) |
| HOLA line format | `buildAndSign`, `createHola` | [hola-agent-authentication.md](https://github.com/discernible-io/idclawserver-idc/blob/main/references/hola-agent-authentication.md) |
| Peer verify | Not included — use API or OpenClaw tool | `POST /api/identity/verify` / `identyclaw_verify_hola` |

**Do not confuse timestamps:** login uses `timestamp_iso` from `GET /api/login/timestamp`. HOLA uses `timestamp` + `noncetsHex` from `GET /api/holanonce16ts`.

---

## Install

Vendored in the OpenClaw plugin (`file:./hola-client`). Standalone:

```bash
npm install @rodit/hola-client
```

---

## API

| Export | Purpose |
| --- | --- |
| `createHola({ nearPrivateKey, jwt, tokenId, ... })` | API session + nonce fetch + local HOLA sign (recommended) |
| `getNonce({ baseUrl, jwt })` | `GET /api/holanonce16ts` (requires API bearer token) |
| `buildAndSign({ recipient, tokenId, timestamp, noncetsHex, privateKey })` | Build standard HOLA line when you already have nonce fields |
| `nearPrivateKeyToSigningSecretKey(nearPrivateKey)` | NEAR `ed25519:` key → tweetnacl secret (stays on host) |
| `buildCanonicalPrefix(...)` | Unsigned uppercase prefix (testing) |
| `parseHola(holaString)` | Format + checksum parse (not full trust / on-chain verify) |
| `computeHolaChecksum(prefix)` | Mod-23 checksum letter |
| `buildCollaborationEnvelope(...)` | `identyclaw.collaboration.v1` task wrapper |
| `parseCollaborationEnvelope(input)` | Parse JSON or ` ```identyclaw ` fenced message |
| `formatSessionsSendMessage(envelope)` | OpenClaw `sessions_send` body with fence |
| `assertCollaborationTrust(envelope, verifyResult)` | Trust decision after API verify |

`privateKey` is a 64-byte tweetnacl Ed25519 secret key. HOLA line signatures use **base32**; API login uses **base64url** over a different message — see server login docs.

---

## Example — full create path

```javascript
const { createHola } = require("@rodit/hola-client");

// jwt_token from POST /api/login — NOT a HOLA line
const { hola } = await createHola({
  baseUrl: "https://api.identyclaw.com",
  jwt: process.env.IDENTYCLAW_JWT,
  nearPrivateKey: process.env.IDENTYCLAW_NEAR_PRIVATE_KEY,
  tokenId: "yourpassportid",
  recipient: "MUNDO"
});
// hola is the wire string to send to a peer or POST /api/testhola
```

## Example — manual nonce + sign

```javascript
const { getNonce, buildAndSign, nearPrivateKeyToSigningSecretKey } = require("@rodit/hola-client");

const { noncetsHex, timestamp } = await getNonce({
  baseUrl: "https://api.identyclaw.com",
  jwt: process.env.IDENTYCLAW_JWT
});
const privateKey = nearPrivateKeyToSigningSecretKey(process.env.IDENTYCLAW_NEAR_PRIVATE_KEY);

const { hola } = buildAndSign({
  recipient: "MUNDO",
  tokenId: "yourpassportid",
  timestamp,
  noncetsHex,
  privateKey
});
```

Peer verification: `POST /api/identity/verify` with `{"hola":"<line>"}` and your API bearer token — or OpenClaw `identyclaw_verify_hola`.

---

## Tests

```bash
npm test
```

---

## License

MIT-0 — see parent plugin [LICENSE](../LICENSE).
