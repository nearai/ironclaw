# Findings from integrating the Ledger device SDKs

Two observations from wiring `@ledgerhq/device-management-kit` and friends into a
transaction-signing flow, written up for Ledger. Both concern **default network
egress**, and both are about the SDKs rather than about our integration.

Neither is a vulnerability in the exploitable sense. Nothing here lets an
attacker sign a transaction, recover a key, or bypass a device prompt. They are
**hardening requests**: defaults that are reasonable for a wallet application
and surprising for an integrator embedding the SDK in a signing path, where the
set of processes allowed to observe or transmit is deliberately small.

We have mitigated both locally. We are reporting them because the mitigations
are the sort a second integrator would have to rediscover, and because one of
them activates through no action of the integrator's own.

**Context for the numbers below:** everything was observed against a clean
install of the published packages, versions as stated, on 2026-07-27. Evidence
is quoted from the shipped `lib/esm` bundles, not from source.

---

## Finding 1 — the WebHID transport reports device errors to Sentry by default

### Affected

| Package | Version | Role |
|---|---|---|
| `@ledgerhq/device-transport-kit-web-hid` | 1.2.4 | declares **and imports** `@sentry/minimal@6.19.7` |
| `@ledgerhq/device-management-kit` | 1.7.1 | declares `@sentry/minimal@6.19.7`; no import found in shipped ESM |

### What we observed

`device-transport-kit-web-hid` imports `@sentry/minimal` and calls
`captureException` on **five paths**, all of them ordinary device-error paths:

`lib/esm/api/transport/WebHidTransport.js` — 4 call sites:

- `getDevices()` — enumerating HID devices fails
- `promptDeviceAccess()` — the user-gesture device picker fails
- `startDiscovering()` — discovery fails
- `updateTransportDiscoveredDevices()` — refresh fails

`lib/esm/api/transport/WebHidApduSender.js` — 1 call site, in the device-open
error path:

```js
const n = new v(e);
throw this.logger.error("Error while opening device", { data: { error: e } }),
  g.captureException(n),
  e;
```

In `@sentry/minimal`, `captureException` is not self-contained — it delegates to
whatever Sentry hub is current in the process. With no hub configured it is
inert.

### Why this is worth changing

The behaviour is **conditional on state the integrator does not set here and may
not associate with signing at all**. An application that calls `Sentry.init()`
anywhere — for unrelated frontend error reporting, or because a framework
template included it — silently begins exporting device-interaction errors from
a transaction-signing flow to a third-party endpoint. Nothing in the signing code
changed; nothing in the SDK call site is visible at the integration boundary.

The payloads are error objects from device interaction, not key material. Our
concern is narrower than "data leak": in a signing path, the set of processes
that may observe activity is a deliberate design decision, and this makes it
depend on an unrelated dependency's configuration.

Two smaller points:

1. **`@sentry/minimal` is end-of-life.** The package was folded into
   `@sentry/core` in Sentry v7; the last stable publish is on the 6.x line
   (the only 7.x artifact is `7.0.0-alpha.1`). A pinned EOL telemetry SDK is
   an odd thing to carry in a security-sensitive dependency tree, independent
   of whether it transmits.
2. **`device-management-kit` declares the dependency but we found no import of
   it in the shipped ESM.** If that is accurate, dropping it from that package's
   manifest would shrink the tree for everyone.

### What would resolve it

In rough order of preference:

1. **Make error reporting opt-in.** Accept a reporter in
   `DeviceManagementKitBuilder` / the transport factory, defaulting to a no-op.
   Integrators who want Sentry pass theirs; integrators who want silence get it
   without patching the dependency graph.
2. **Use a dedicated hub rather than the ambient one**, so the SDK's reporting
   is never switched on by an unrelated `Sentry.init()`.
3. **At minimum, document it** — a line in the integration guide saying the
   WebHID transport reports to the ambient Sentry hub would let integrators make
   an informed choice. Today the only way to discover it is to read the bundle.

Independently: move off `@sentry/minimal`, and drop the declaration from
`device-management-kit` if it is genuinely unused.

### What we did

Aliased `@sentry/minimal` to a local no-op via a pnpm override, with the
integrator-facing surface stubbed so a future SDK version calling a different
member cannot throw inside our ceremony. Device errors are not lost — our
ceremony surfaces each as its own outcome, which is where a user-facing failure
belongs anyway.

We also added a test asserting every Ledger package resolves the stub, because a
pnpm override is one line that a dependency bump or a lockfile regeneration can
silently drop.

---

## Finding 2 — `context-module` enables four remote endpoints by default, including blind-signing telemetry

### Affected

`@ledgerhq/context-module` 2.3.1.

### What we observed

`DEFAULT_CONFIG`:

```json
{
  "cal":                  { "url": "https://global.api.prd.ledger.com/cal/v1",
                            "mode": "prod", "branch": "main" },
  "web3checks":           { "url": "https://global.api.prd.ledger.com/transaction-checks/v3" },
  "metadataServiceDomain":{ "url": "https://nft.api.live.ledger.com" },
  "reporter":             { "url": "https://blind-signing.api.ledger.com/ingest/v1" },
  "datasource":           { "proxy": "default" },
  "appSource":            "third-party"
}
```

Four remote hosts, all reachable by default. `ContextModuleBuilder` exposes
setters for each (`setCalConfig`, `setWeb3ChecksConfig`,
`setMetadataServiceConfig`, `setReporterConfig`), so they can be repointed — but
the default is on, and an integrator who never opens the config has all four.

### Why this is worth changing

The `cal` endpoint is the point of the module and needs no defence. The other
three are worth separating:

- **`reporter` → `blind-signing.api.ledger.com/ingest/v1`.** This exists to
  report blind-signing events. For an integrator who **never blind-signs** — in
  our case, a transaction without a clear-signing descriptor never reaches the
  device at all — it has nothing legitimate to report, yet it is configured and
  reachable. An always-on telemetry endpoint in a signing bundle is the kind of
  thing that should be opt-in even when it would never fire.
- **`web3checks` and `metadataServiceDomain`** are useful features, but they are
  *additional* remote calls made while a user is deciding whether to sign, and
  an integrator may reasonably want to decide about each one.

A related point about CSP, which is how many integrators will first encounter
this: an app with a strict `connect-src` will simply see these blocked, which
looks like a bug and gets diagnosed as one. Silently blocked and deliberately
disabled are also *not equivalent* — the first leaves a request that begins
working the day someone loosens the CSP for an unrelated reason.

### What would resolve it

- **Default the reporter off**, or gate it behind an explicit opt-in. It is
  telemetry, and it is the one endpoint in the set with no functional role in
  producing a signature.
- **Document the full endpoint list prominently** in the integration guide, with
  the `connect-src` entries an integrator needs. Right now the reliable way to
  learn what the module contacts is to print `DEFAULT_CONFIG`.
- Consider a single `setOfflineMode()` / explicit-allowlist entry point, so an
  integrator can say "only `cal`, and only through my proxy" in one call rather
  than four.

### What we did

Repointed all four. `cal` goes to a same-origin backend proxy — our SPA has a
zero-remote-origins CSP, so the browser cannot reach Ledger's CAL directly and
the backend fetches descriptors on its behalf, over HTTPS only. The other three
point at an inert same-origin path.

The allowlist has exactly one entry, and it is your own default:

```
global.api.prd.ledger.com
```

It is a compile-time constant, not configuration. An operator selects *which*
allowed upstream to use; they cannot add one via the environment. The reasoning
is that a descriptor decides what the device shows a human who is about to sign,
so an attacker who could repoint this would not need to touch the transaction at
all — only to change what the device says it is. **If descriptors should be
fetched from a different host (per-region, staging, or a CDN edge), tell us and
we will add it to the allowlist** — we would rather extend a constant than make
the host a free-text setting.

---

## Questions we could not answer ourselves

Separate from the findings, three things we could not verify without hardware or
your live services. All are marked unverified in our code.

1. **CAL request shape.** We query
   `/dapps?chain_id=&contract=&selector=&output=descriptor` against the SDK's
   default base URL. Is that the correct shape for descriptor lookup by
   `(chain, contract, selector)`?
2. **DMK state and error mapping.** We map device conditions to user-facing
   states — locked, wrong app open, user rejection (we key on `0x6985` and on a
   `UserRejectedError` tag), disconnect mid-sign. Do those match what DMK
   actually emits? Getting this wrong tells a user to unlock a device that is
   really running the wrong app.
3. **`@ledgerhq/coin-evm` outside Ledger Live.** We run `createApi` in plain
   Node with a hand-supplied `CurrenciesResolver`, for transaction crafting and
   broadcast in a sidecar. Is that a supported use, or are we relying on
   something incidental?

## One design choice you may disagree with

When no ERC-7730 descriptor covers a transaction, we **block signing entirely**:
the device is never contacted, there is no blind-sign button, and no override
flag exists in any build.

The reasoning is that a device showing a bare hash gives the human nothing to
check, so the ceremony's security value drops to zero while its *appearance* is
unchanged — and a ceremony that looks like verification but isn't seems worse
than a refusal, because it trains the user to click through. This is stricter
than the SDK's default posture and we would be interested in whether you think
it is the right call.

---

## Reproducing

```bash
npm view @ledgerhq/device-transport-kit-web-hid@1.2.4 dependencies
npm view @ledgerhq/context-module@2.3.1 dependencies
node -e "console.log(require('@ledgerhq/context-module').DEFAULT_CONFIG)"
grep -c 'captureException' node_modules/@ledgerhq/device-transport-kit-web-hid/lib/esm/api/transport/WebHidTransport.js
```

## Contact

Filed from the IronClaw integration at `nearai/ironclaw`. The mitigations
described are in PR nearai/ironclaw#6672 — `pnpm-workspace.yaml` for the
override, `crates/ironclaw_webui/frontend/vendor/sentry-minimal-noop/` for the
stub, and `src/lib/ledger/dmk-adapter.ts` for the endpoint lockdown.
