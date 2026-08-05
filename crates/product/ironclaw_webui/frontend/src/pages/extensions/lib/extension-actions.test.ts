// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import {
  extensionLifecycleState,
  extensionIsActive,
  primaryExtensionAction,
} from "./extension-actions";

const notionRef = { kind: "extension", id: "notion" };
const channelSurfaces = [{ kind: "channel", inbound: true, outbound: true }];
const toolSurfaces = [{ kind: "tool" }];

test("primaryExtensionAction keeps incomplete MCP extensions in setup", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: notionRef,
      runtime: "mcp",
      surfaces: toolSurfaces,
      installation_state: "setup_needed",
    }),
    "configure",
  );
});

test("primaryExtensionAction configures setup-needed channels generically", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      runtime: "first_party",
      surfaces: channelSurfaces,
      installation_state: "setup_needed",
    }),
    "configure",
  );
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "telegram" },
      runtime: "wasm",
      surfaces: channelSurfaces,
      installation_state: "setup_needed",
    }),
    "configure",
  );
});

test("primaryExtensionAction returns no action for active extensions", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: notionRef,
      runtime: "mcp",
      surfaces: toolSurfaces,
      installation_state: "active",
    }),
    null,
  );
});

test("extensionLifecycleState trusts the caller-scoped projection", () => {
  assert.equal(
    extensionLifecycleState({
      package_ref: { kind: "extension", id: "slack" },
      installation_state: "setup_needed",
    }),
    "setup_needed",
  );
});

test("extensionLifecycleState accepts only the authoritative wire field", () => {
  assert.equal(extensionLifecycleState({ installation_state: "active" }), "active");
  assert.equal(extensionLifecycleState({ installation_state: "setup_needed" }), "setup_needed");
  assert.equal(extensionLifecycleState({}), "uninstalled");
  assert.equal(extensionLifecycleState({ installation_state: "ready" }), "uninstalled");
});

test("extensionIsActive requires authoritative active state", () => {
  assert.equal(extensionIsActive({ installation_state: "active" }), true);
  assert.equal(extensionIsActive({ installation_state: "setup_needed" }), false);
  assert.equal(extensionIsActive({}), false);
});
