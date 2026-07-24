import assert from "node:assert/strict";
import { test } from "vitest";

import {
  RUNTIME_LABELS,
  STATE_LABELS,
  STATE_TONES,
  authAccountNeedsReconnect,
  authAccountReasonLabelKey,
  channelConnection,
  channelSurface,
  extensionSurfaces,
  hasAuthSurface,
  hasChannelSurface,
  hasToolSurface,
  isWebGeneratedCodeConnection,
  primaryAuthAccount,
} from "./extensions-schema";

test("extensionSurfaces returns the wire surfaces and tolerates missing ones", () => {
  const surfaces = [{ kind: "channel", inbound: true, outbound: true }];
  assert.equal(extensionSurfaces({ surfaces }), surfaces);
  assert.deepEqual(extensionSurfaces({}), []);
  assert.deepEqual(extensionSurfaces(null), []);
  assert.deepEqual(extensionSurfaces(undefined), []);
});

test("hasChannelSurface keys off a declared channel surface only", () => {
  assert.equal(
    hasChannelSurface({
      surfaces: [{ kind: "channel", inbound: true, outbound: true }],
    }),
    true,
  );
  // Null surface entries are tolerated, not fatal.
  assert.equal(
    hasChannelSurface({ surfaces: [null, { kind: "channel", inbound: true, outbound: true }] }),
    true,
  );
  assert.equal(hasChannelSurface({ surfaces: [{ kind: "tool" }, { kind: "auth" }] }), false);
  assert.equal(hasChannelSurface({ surfaces: [] }), false);
  assert.equal(hasChannelSurface({}), false);
  assert.equal(hasChannelSurface(undefined), false);
});

test("hasAuthSurface keys off the typed auth surface", () => {
  assert.equal(hasAuthSurface({ surfaces: [{ kind: "auth" }] }), true);
  assert.equal(
    hasAuthSurface({ surfaces: [{ kind: "tool" }, { kind: "channel" }] }),
    false,
  );
  assert.equal(hasAuthSurface({}), false);
});

test("hasToolSurface keys off a declared tool surface only", () => {
  assert.equal(
    hasToolSurface({
      surfaces: [{ kind: "tool" }, { kind: "channel", inbound: true, outbound: true }],
    }),
    true,
  );
  assert.equal(
    hasToolSurface({ surfaces: [{ kind: "channel", inbound: true, outbound: true }] }),
    false,
  );
  assert.equal(hasToolSurface({ surfaces: [] }), false);
  assert.equal(hasToolSurface({}), false);
  assert.equal(hasToolSurface(undefined), false);
});

test("channelSurface and channelConnection extract the typed channel surface", () => {
  const connection = { channel: "telegram", strategy: "web_generated_code" };
  const surface = {
    kind: "channel",
    inbound: true,
    outbound: true,
    connection,
  };
  const item = {
    package_ref: { id: "telegram" },
    surfaces: [{ kind: "tool" }, { kind: "auth" }, surface],
  };

  assert.equal(channelSurface(item), surface);
  assert.equal(channelSurface({ surfaces: [{ kind: "tool" }, { kind: "auth" }] }), null);
  assert.equal(channelSurface({}), null);

  assert.equal(channelConnection(item), connection);
  assert.equal(
    channelConnection({ surfaces: [{ kind: "channel", inbound: true, outbound: false }] }),
    null,
    "a channel surface without a connect affordance yields no connection",
  );

  assert.equal(isWebGeneratedCodeConnection(connection), true);
  assert.equal(isWebGeneratedCodeConnection({ strategy: "oauth" }), false);
  assert.equal(isWebGeneratedCodeConnection({ strategy: "admin_managed_channels" }), false);
  assert.equal(isWebGeneratedCodeConnection(null), false);
});

test("RUNTIME_LABELS covers exactly the honest runtime wire values", () => {
  assert.deepEqual(Object.keys(RUNTIME_LABELS).sort(), [
    "first_party",
    "mcp",
    "script",
    "system",
    "wasm",
  ]);
});

test("STATE_TONES/STATE_LABELS expose only the two listed public lifecycle states", () => {
  // Absence from the installed response is `uninstalled`; a present extension
  // is either waiting on manifest-declared setup or active. Internal
  // install/discovery/publication checkpoints never become card states.
  const expectedKeys = ["active", "setup_needed"];
  assert.deepEqual(Object.keys(STATE_LABELS).sort(), expectedKeys);
  assert.deepEqual(Object.keys(STATE_TONES).sort(), expectedKeys);

  for (const state of expectedKeys) {
    assert.ok(STATE_LABELS[state], `${state} must have a label (known-state check)`);
    assert.ok(STATE_TONES[state], `${state} must have a tone, not the muted default`);
  }
  assert.equal(STATE_TONES.active, "success");
  assert.equal(
    STATE_TONES.setup_needed,
    "warning",
    "setup-needed extensions must use the yellow warning pill",
  );

  for (const dead of [
    "auth_required",
    "configured",
    "disabled",
    "failed",
    "installed",
    "setup_required",
    "unsupported",
    "activating",
    "deactivating",
    "removing",
    "removal_pending",
    "removed",
  ]) {
    assert.equal(STATE_LABELS[dead], undefined, `${dead} is retired and must not be a known state`);
    assert.equal(STATE_TONES[dead], undefined, `${dead} is retired and must not be a known state`);
  }
});

test("primaryAuthAccount/authAccountNeedsReconnect read the §6.3 account state and last_error (G4)", () => {
  const expired = {
    auth_accounts: [
      {
        vendor: "acme",
        accounts: [
          { account_id: "acme", state: "expired", last_error: "refresh_failed", is_default: true },
        ],
      },
    ],
  };
  assert.equal(primaryAuthAccount(expired)?.state, "expired");
  assert.equal(primaryAuthAccount(expired)?.last_error, "refresh_failed");
  assert.equal(authAccountNeedsReconnect(expired), true, "expired account needs reconnect");

  // A `disconnected` account carrying a typed reason means a live connection
  // broke (revoked grant, missing credential, a prior auth attempt that
  // failed/expired) — that also needs reconnect, distinct from a fresh
  // never-connected extension.
  const revoked = {
    auth_accounts: [
      {
        vendor: "acme",
        accounts: [
          { account_id: "acme", state: "disconnected", last_error: "grant_revoked", is_default: true },
        ],
      },
    ],
  };
  assert.equal(authAccountNeedsReconnect(revoked), true, "a revoked grant needs reconnect");

  const missingCredential = {
    auth_accounts: [
      {
        vendor: "acme",
        accounts: [
          {
            account_id: "acme",
            state: "disconnected",
            last_error: "credential_missing",
            is_default: true,
          },
        ],
      },
    ],
  };
  assert.equal(
    authAccountNeedsReconnect(missingCredential),
    true,
    "a missing credential needs reconnect",
  );

  // A fresh, never-connected account is `disconnected` with NO last_error —
  // that stays a plain first-time Connect, not a reconnect.
  const freshDisconnected = {
    auth_accounts: [
      { vendor: "acme", accounts: [{ account_id: "acme", state: "disconnected", is_default: true }] },
    ],
  };
  assert.equal(
    authAccountNeedsReconnect(freshDisconnected),
    false,
    "a fresh disconnected account with no reason is a first-time connect",
  );

  const connected = {
    auth_accounts: [{ vendor: "acme", accounts: [{ account_id: "acme", state: "connected" }] }],
  };
  assert.equal(
    authAccountNeedsReconnect(connected),
    false,
    "a healthy connected account does not need reconnect",
  );

  // No accounts / non-channel extension → nothing to reconnect.
  assert.equal(primaryAuthAccount({}), null);
  assert.equal(authAccountNeedsReconnect({}), false);
  assert.equal(authAccountNeedsReconnect({ auth_accounts: [] }), false);
});

test("authAccountReasonLabelKey maps every §6.3 last_error to a distinct i18n key (G4)", () => {
  // Each of the seven typed reasons must render distinct copy; `refresh_failed`
  // reuses the existing expiry key since it is the reason an `expired` account
  // always carries. There is no `revoking` entry: disconnect/removal delete
  // the account synchronously (overview §6.3), so no in-progress revoking
  // reason is ever produced.
  const cases = {
    flow_expired: "extensions.accountFlowExpired",
    vendor_denied: "extensions.accountVendorDenied",
    exchange_failed: "extensions.accountExchangeFailed",
    refresh_failed: "extensions.accountExpired",
    grant_revoked: "extensions.accountRevoked",
    validation_probe_failed: "extensions.accountValidationFailed",
    credential_missing: "extensions.accountCredentialMissing",
  };
  const seen = new Set();
  for (const [lastError, expectedKey] of Object.entries(cases)) {
    const key = authAccountReasonLabelKey({ state: "disconnected", last_error: lastError });
    assert.equal(key, expectedKey, `${lastError} must map to ${expectedKey}`);
    seen.add(key);
  }
  assert.equal(seen.size, Object.keys(cases).length, "every last_error must render distinct copy");

  // No typed reason (e.g. a bare `expired` state) falls back to the generic
  // expiry copy rather than throwing or rendering `undefined`.
  assert.equal(authAccountReasonLabelKey({ state: "expired" }), "extensions.accountExpired");
  assert.equal(authAccountReasonLabelKey(null), "extensions.accountExpired");
});
