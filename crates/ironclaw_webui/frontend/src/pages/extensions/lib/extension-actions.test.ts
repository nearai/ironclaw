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

test("primaryExtensionAction opens configuration for OAuth-required setup", () => {
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

test("primaryExtensionAction routes legacy onboarding substates to configuration", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      runtime: "first_party",
      surfaces: channelSurfaces,
      onboarding_state: "pairing_required",
    }),
    "configure",
    "channel surface + pairing_required should route to its generic setup UI",
  );
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      runtime: "first_party",
      surfaces: channelSurfaces,
      onboarding_state: "pairing",
    }),
    "configure",
    "channel surface + pairing should route to its generic setup UI",
  );
  assert.equal(
    primaryExtensionAction({
      package_ref: { kind: "extension", id: "slack" },
      runtime: "first_party",
      surfaces: channelSurfaces,
      installation_state: "setup_needed",
    }),
    "configure",
    "channel surface + setup_needed should hand off to generic setup UI",
  );
});

test("primaryExtensionAction returns no action for active extensions", () => {
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

test("extensionLifecycleState trusts the collapsed caller-scoped projection", () => {
  assert.equal(
    extensionLifecycleState({
      package_ref: { kind: "extension", id: "slack" },
      kind: "wasm_tool",
      active: true,
      authenticated: false,
      needs_setup: true,
      has_auth: true,
      installation_state: "setup_needed",
    }),
    "setup_needed",
  );
});

test("extensionIsActive accepts card payload lifecycle fields", () => {
  assert.equal(extensionIsActive({ active: true }), true);
  assert.equal(extensionIsActive({ installationState: "ready" }), false);
  assert.equal(extensionIsActive({ onboardingState: "ready" }), false);
  assert.equal(extensionIsActive({ onboardingState: "auth_required" }), false);
  assert.equal(
    extensionIsActive({ installationState: "setup_needed", onboardingState: "active" }),
    false,
    "the canonical installation state must win over stale onboarding metadata",
  );
});
