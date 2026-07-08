import assert from "node:assert/strict";
import { test } from "vitest";

import { isChannelExtensionKind } from "./extensions-schema";

test("isChannelExtensionKind matches both channel extension kinds", () => {
  assert.equal(isChannelExtensionKind("channel"), true);
  assert.equal(isChannelExtensionKind("wasm_channel"), true);
  assert.equal(isChannelExtensionKind("mcp_server"), false);
  assert.equal(isChannelExtensionKind(undefined), false);
});
