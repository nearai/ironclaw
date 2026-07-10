// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import {
  extensionLifecycleState,
  extensionIsActive,
  primaryExtensionAction,
  setupReadyForActivation,
} from "./extension-actions";

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
    "kind:channel + installed should return null (configure/setup owns it)",
  );
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      kind: "channel",
      activation_status: "installed",
    }),
    null,
    "kind:channel + installed should hand off to channel configure/setup UI",
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

test("extensionLifecycleState does not call active unauthenticated setup active", () => {
  assert.equal(
    extensionLifecycleState({
      package_ref: { kind: "extension", id: "slack" },
      kind: "wasm_tool",
      active: true,
      authenticated: false,
      needs_setup: true,
      has_auth: true,
      activation_status: "active",
    }),
    "auth_required",
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
