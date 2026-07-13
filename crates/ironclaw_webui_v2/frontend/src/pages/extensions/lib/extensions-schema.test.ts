import assert from "node:assert/strict";
import { test } from "vitest";

import {
  RUNTIME_LABELS,
  channelConnection,
  channelSurface,
  connectsViaOauth,
  extensionSurfaces,
  hasChannelSurface,
  hasToolSurface,
  isInboundProofCodeConnection,
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
  const connection = { channel: "telegram", strategy: "inbound_proof_code" };
  const surface = {
    kind: "channel",
    inbound: true,
    outbound: true,
    connected: false,
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

  assert.equal(isInboundProofCodeConnection(connection), true);
  assert.equal(isInboundProofCodeConnection({ strategy: "oauth" }), false);
  assert.equal(isInboundProofCodeConnection({ strategy: "admin_managed_channels" }), false);
  assert.equal(isInboundProofCodeConnection(null), false);
});

test("connectsViaOauth derives from the wire connection strategy or oauth setup secrets", () => {
  const oauthConnected = {
    surfaces: [
      { kind: "channel", inbound: true, outbound: true, connection: { strategy: "oauth" } },
    ],
  };
  const proofCodeConnected = {
    surfaces: [
      {
        kind: "channel",
        inbound: true,
        outbound: true,
        connection: { strategy: "inbound_proof_code" },
      },
    ],
  };
  const bare = { surfaces: [{ kind: "channel", inbound: true, outbound: true }] };

  assert.equal(connectsViaOauth(oauthConnected), true);
  assert.equal(connectsViaOauth(proofCodeConnected), false);
  assert.equal(connectsViaOauth(bare), false);
  assert.equal(connectsViaOauth(undefined), false);

  // An oauth-kind setup secret marks the extension OAuth-connecting even when
  // the surface carries no connection requirement.
  const oauthSecret = { name: "vendor_oauth", setup: { kind: "oauth" } };
  const manualSecret = { name: "vendor_token", setup: { kind: "manual_token" } };
  assert.equal(connectsViaOauth(bare, [oauthSecret]), true);
  assert.equal(connectsViaOauth(bare, [manualSecret]), false);
  assert.equal(connectsViaOauth(bare, [null]), false);
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
