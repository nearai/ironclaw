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
const channelSurfaces = [{ kind: "channel", inbound: true, outbound: true }];
const toolSurfaces = [{ kind: "tool" }];

test("primaryExtensionAction opens configuration before OAuth-required activation", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: notionRef,
      runtime: "mcp",
      surfaces: toolSurfaces,
      onboarding_state: "auth_required",
    }),
    "configure",
  );
});

test("primaryExtensionAction activates configured inactive MCP extensions", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: notionRef,
      runtime: "mcp",
      surfaces: toolSurfaces,
      installation_state: "installed",
    }),
    "activate",
  );
});

test("primaryExtensionAction suppresses activation for channel-surface extensions", () => {
  // The suppression is surface-driven, not runtime-driven: a first-party and a
  // WASM channel extension behave identically.
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      runtime: "first_party",
      surfaces: channelSurfaces,
      installation_state: "installed",
    }),
    null,
  );
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "telegram" },
      runtime: "wasm",
      surfaces: channelSurfaces,
      installation_state: "installed",
    }),
    null,
  );
});

test("primaryExtensionAction suppresses Activate for channel surfaces in pairing states", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      runtime: "first_party",
      surfaces: channelSurfaces,
      onboarding_state: "pairing_required",
    }),
    null,
    "channel surface + pairing_required should return null (pairing section owns it)",
  );
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      runtime: "first_party",
      surfaces: channelSurfaces,
      onboarding_state: "pairing",
    }),
    null,
    "channel surface + pairing should return null (configure/setup owns it)",
  );
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      runtime: "first_party",
      surfaces: channelSurfaces,
      installation_state: "installed",
    }),
    null,
    "channel surface + installed should hand off to channel configure/setup UI",
  );
});

test("primaryExtensionAction hides activation for active extensions", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: notionRef,
      runtime: "mcp",
      surfaces: toolSurfaces,
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
      installation_state: "active",
    }),
    "auth_required",
  );
});

test("extensionIsActive accepts card payload lifecycle fields", () => {
  assert.equal(extensionIsActive({ active: true }), true);
  assert.equal(extensionIsActive({ installationState: "ready" }), true);
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
