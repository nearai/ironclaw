import assert from "node:assert/strict";
import { test } from "vitest";

import {
  RUNTIME_LABELS,
  extensionSurfaces,
  hasChannelSurface,
  hasToolSurface,
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

test("RUNTIME_LABELS covers exactly the honest runtime wire values", () => {
  assert.deepEqual(Object.keys(RUNTIME_LABELS).sort(), [
    "first_party",
    "mcp",
    "script",
    "system",
    "wasm",
  ]);
});
