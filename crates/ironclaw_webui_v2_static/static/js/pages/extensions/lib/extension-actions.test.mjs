import assert from "node:assert/strict";
import test from "node:test";

import {
  extensionIsActive,
  primaryExtensionAction,
  setupReadyForActivation,
} from "./extension-actions.js";

const notionRef = { kind: "extension", id: "notion" };

test("primaryExtensionAction opens configuration before OAuth-required activation", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: notionRef,
      kind: "mcp_server",
      onboarding_state: "auth_required",
    }),
    "configure",
  );
});

test("primaryExtensionAction activates configured inactive MCP extensions", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: notionRef,
      kind: "mcp_server",
      activation_status: "installed",
    }),
    "activate",
  );
});

test("primaryExtensionAction hides activation for active extensions", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: notionRef,
      kind: "mcp_server",
      active: true,
    }),
    null,
  );
});

test("extensionIsActive accepts card payload lifecycle fields", () => {
  assert.equal(extensionIsActive({ active: true }), true);
  assert.equal(extensionIsActive({ activationStatus: "ready" }), true);
  assert.equal(extensionIsActive({ onboardingState: "auth_required" }), false);
});

test("setupReadyForActivation waits until all setup secrets are provided", () => {
  assert.equal(
    setupReadyForActivation({
      secrets: [{ provided: true }, { provided: true }],
      fields: [],
    }),
    true,
  );
  assert.equal(
    setupReadyForActivation({
      secrets: [{ provided: true }, { provided: false }],
      fields: [],
    }),
    false,
  );
  assert.equal(
    setupReadyForActivation({
      secrets: [{ provided: true }],
      fields: [{ name: "workspace" }],
    }),
    false,
  );
  assert.equal(
    setupReadyForActivation({
      extension: { active: true },
      secrets: [{ provided: true }],
      fields: [],
    }),
    false,
  );
});
