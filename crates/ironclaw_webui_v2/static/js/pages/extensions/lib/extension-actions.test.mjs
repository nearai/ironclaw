import assert from "node:assert/strict";
import test from "node:test";

import {
  extensionHasCredentialConfigurationSurface,
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

test("primaryExtensionAction suppresses activation for channel-surface extensions", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      kind: "channel",
      activation_status: "installed",
    }),
    null,
  );
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "telegram" },
      kind: "wasm_channel",
      activation_status: "installed",
    }),
    null,
  );
});

test("primaryExtensionAction suppresses Activate for channel kind in pairing states", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      kind: "channel",
      onboarding_state: "pairing_required",
    }),
    null,
    "kind:channel + pairing_required should return null (pairing section owns it)",
  );
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      kind: "channel",
      onboarding_state: "pairing",
    }),
    null,
    "kind:channel + pairing should return null (pairing section owns it)",
  );
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      kind: "channel",
      activation_status: "installed",
    }),
    null,
    "kind:channel + installed should hand off to channel setup/pairing UI",
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

test("extensionHasCredentialConfigurationSurface ignores activation-only setup hints", () => {
  assert.equal(
    extensionHasCredentialConfigurationSurface({
      package_ref: { kind: "extension", id: "portfolio" },
      kind: "wasm_tool",
      activation_status: "installed",
      needs_setup: true,
      has_auth: false,
      authenticated: false,
    }),
    false,
    "installed no-auth tools may need activation, but not a Configure form",
  );
  assert.equal(
    extensionHasCredentialConfigurationSurface({
      package_ref: notionRef,
      kind: "mcp_server",
      activation_status: "installed",
      has_auth: true,
      authenticated: false,
    }),
    false,
    "unauthenticated auth-capable extensions should use auth/setup states before Configure",
  );
  assert.equal(
    extensionHasCredentialConfigurationSurface({
      package_ref: notionRef,
      kind: "mcp_server",
      onboarding_state: "setup_required",
      has_auth: true,
      authenticated: false,
    }),
    true,
  );
  assert.equal(
    extensionHasCredentialConfigurationSurface({
      package_ref: notionRef,
      kind: "mcp_server",
      activation_status: "active",
      has_auth: true,
      authenticated: true,
    }),
    true,
    "configured credential extensions keep a reconfigure affordance",
  );
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
