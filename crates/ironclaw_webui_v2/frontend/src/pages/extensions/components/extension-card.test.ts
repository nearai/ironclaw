// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

// ---------------------------------------------------------------------------
// Source munging — strip ES module imports, rewrite exports, inject test shim
// ---------------------------------------------------------------------------

/**
 * Strip all import declarations (single-line and multi-line block imports)
 * from a JS source string and rewrite "export function" → "function".
 * Multi-line imports are spans from a line starting with "import {" through
 * the closing `} from "..."` line.
 */
function stripImports(source) {
  const lines = source.split("\n");
  const out = [];
  let inBlockImport = false;
  for (const line of lines) {
    if (inBlockImport) {
      // End of block import is the line matching `} from "..."`
      if (/^\s*\}/.test(line) && /from\s+["']/.test(line)) {
        inBlockImport = false;
      }
      // Skip all lines inside (and including the closing line of) a block import
      continue;
    }
    if (line.startsWith("import ")) {
      // Single-line import: skip entirely
      // Multi-line import: starts with "import {" without closing "}" on same line
      if (line.includes("{") && !line.includes("}")) {
        inBlockImport = true;
      }
      continue;
    }
    out.push(line.replace(/^export function /, "function "));
  }
  return out.join("\n");
}

function extensionCardSourceForTest() {
  const source = readFileSync(new URL("./extension-card.tsx", import.meta.url), "utf8");
  return (
    stripImports(source) +
    "\nglobalThis.__testExports = { ExtensionCard, RegistryCard };"
  );
}

// ---------------------------------------------------------------------------
// VM context helpers
// ---------------------------------------------------------------------------

/**
 * Build a minimal vm context that satisfies all dependencies imported by
 * extension-card.tsx:
 *   - React (useState, useRef, useEffect)
 *   - useT i18n stub
 *   - Badge, Button, Icon design-system stubs
 *   - KIND_LABELS, STATE_TONES, STATE_LABELS, isChannelExtensionKind from extensions-schema
 *   - primaryExtensionAction from extension-actions
 */
function makeContext() {
  // Minimal React stub — useState returns [initial, noop]; refs and effects are ignored.
  const React = {
    useState: (initial) => [initial, () => {}],
    useRef: () => ({ current: null }),
    useEffect: () => {},
  };

  // i18n stub — returns the key suffix after the last dot so labels are
  // predictable in assertions (e.g. "extensions.reconfigure" → "reconfigure").
  function useT() {
    return (key) => key.split(".").pop();
  }

  // Design-system component stubs — identity functions; their exact shape
  // doesn't matter because we only inspect the overflowActions values array.
  function Badge() {}
  function Button() {}
  function Icon() {}

  // Inline isChannelExtensionKind from extensions-schema.ts (exact copy).
  function isChannelExtensionKind(kind) {
    return kind === "wasm_channel" || kind === "channel";
  }

  const KIND_LABELS = {
    wasm_tool: "WASM Tool",
    wasm_channel: "Channel",
    channel: "Channel",
    mcp_server: "MCP Server",
    first_party: "First-party",
    system: "System",
    channel_relay: "Relay",
  };

  const STATE_TONES = {
    active: "success",
    ready: "success",
    pairing_required: "warning",
    pairing: "warning",
    auth_required: "warning",
    setup_required: "muted",
    failed: "danger",
    installed: "muted",
  };

  const STATE_LABELS = {
    active: "active",
    ready: "ready",
    pairing_required: "pairing",
    pairing: "pairing",
    auth_required: "auth needed",
    setup_required: "setup needed",
    failed: "failed",
    installed: "installed",
  };

  // Inline primaryExtensionAction from extension-actions.ts (exact copy).
  function extensionLifecycleState(ext) {
    const onboardingState = ext?.onboarding_state || ext?.onboardingState;
    if (onboardingState) {
      return onboardingState;
    }
    if (ext?.needs_setup === true && ext?.authenticated === false) {
      return ext?.has_auth ? "auth_required" : "setup_required";
    }
    return ext?.activation_status || ext?.activationStatus || (ext?.active ? "active" : "installed");
  }

  function primaryExtensionAction(ext) {
    const state = extensionLifecycleState(ext);
    if (!ext?.package_ref || state === "active" || state === "ready") {
      return null;
    }

    if (state === "auth_required" || state === "setup_required") {
      return "configure";
    }

    return isChannelExtensionKind(ext?.kind) ? null : "activate";
  }

  return {
    globalThis: {},
    React,
    useT,
    Badge,
    Button,
    Icon,
    isChannelExtensionKind,
    KIND_LABELS,
    STATE_TONES,
    STATE_LABELS,
    extensionLifecycleState,
    primaryExtensionAction,
  };
}

/**
 * Render ExtensionCard with the given ext prop and return the rendered tree.
 * onConfigure / onActivate / onRemove are no-op stubs.
 */
function renderExtensionCard(ext) {
  const context = makeContext();
  vm.runInNewContext(extensionCardSourceForTest(), context);
  const { ExtensionCard } = context.globalThis.__testExports;
  return ExtensionCard({
    ext,
    onActivate() {},
    onConfigure() {},
    onRemove() {},
    isBusy: false,
  });
}

// ---------------------------------------------------------------------------
// Tree-walking helpers (matching style from channels-tab.test.ts)
// ---------------------------------------------------------------------------

/**
 * Walk the rendered tree and return all values arrays from nodes whose
 * rendered.values array contains `component` as a direct element.
 * Each such node is a TSX component invocation for OverflowMenu,
 * where values[0] === OverflowMenu and values[1] === the actions array.
 */
function findComponentNodes(rendered, component) {
  const results = [];
  if (!rendered || typeof rendered !== "object") return results;
  if (Array.isArray(rendered)) {
    for (const v of rendered) results.push(...findComponentNodes(v, component));
    return results;
  }
  if (Array.isArray(rendered.values)) {
    for (const v of rendered.values) results.push(...findComponentNodes(v, component));
    // Check if this node is a component invocation for `component`.
    if (rendered.values[0] === component) {
      results.push(rendered);
    }
  }
  return results;
}

/**
 * Given the ExtensionCard rendered tree, extract the overflowActions array
 * passed to OverflowMenu. The test JSX factory captures that component
 * invocation as:
 *   { strings: [...], values: [OverflowMenu, overflowActions, isBusy] }
 */
function extractOverflowActions(rendered, OverflowMenuRef) {
  const nodes = findComponentNodes(rendered, OverflowMenuRef);
  if (nodes.length === 0) return null;
  // values[0] = OverflowMenu component ref, values[1] = actions array
  return nodes[0].values[1];
}

function renderedContainsValue(rendered, expected) {
  if (rendered === expected) return true;
  if (!rendered || typeof rendered !== "object") return false;
  if (Array.isArray(rendered)) {
    return rendered.some((value) => renderedContainsValue(value, expected));
  }
  if (Array.isArray(rendered.values)) {
    return rendered.values.some((value) => renderedContainsValue(value, expected));
  }
  return false;
}

// ---------------------------------------------------------------------------
// Locate the OverflowMenu function reference in a fresh context so we can
// compare against it inside rendered trees.  We need it from the same source
// evaluation that produced the tree — we get it by running the source once,
// grabbing the OverflowMenu reference from inside ExtensionCard's closure.
//
// The cleanest approach: extend __testExports to include OverflowMenu.
// We do this by patching the source shim.
// ---------------------------------------------------------------------------

function extensionCardSourceWithInternals() {
  const source = readFileSync(new URL("./extension-card.tsx", import.meta.url), "utf8");
  return (
    stripImports(source) +
    "\nglobalThis.__testExports = { ExtensionCard, RegistryCard, OverflowMenu };"
  );
}

function renderExtensionCardWithInternals(ext) {
  const context = makeContext();
  vm.runInNewContext(extensionCardSourceWithInternals(), context);
  const { ExtensionCard, OverflowMenu } = context.globalThis.__testExports;
  const rendered = ExtensionCard({
    ext,
    onActivate() {},
    onConfigure() {},
    onRemove() {},
    isBusy: false,
  });
  return { rendered, OverflowMenu };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test("card class keeps grid siblings at natural height", () => {
  const rendered = renderExtensionCard({
    package_ref: { id: "telegram" },
    kind: "channel",
    display_name: "Telegram",
  });
  const cardClass = rendered.values[0];

  assert.ok(
    cardClass.includes("self-start"),
    "CARD should align itself to the top of its grid area",
  );
  assert.ok(!cardClass.includes("h-full"), "CARD must not stretch to grid row height");
});

test("installed channel card omits generic Activate while installed MCP card keeps it", () => {
  const channel = renderExtensionCard({
    package_ref: { id: "slack" },
    kind: "channel",
    activation_status: "installed",
    display_name: "Slack",
  });
  assert.equal(
    renderedContainsValue(channel, "Activate"),
    false,
    "Slack-style channels should use configure/setup instead of generic activation",
  );

  const mcp = renderExtensionCard({
    package_ref: { id: "github" },
    kind: "mcp_server",
    activation_status: "installed",
    display_name: "GitHub",
  });
  assert.equal(
    renderedContainsValue(mcp, "activate"),
    true,
    "non-channel extensions should still expose their normal activation action",
  );
});

test("setup-required primary action reads Connect for a channel and Configure for a credential extension", () => {
  // A freshly-installed channel connects (pairs); a credential extension like
  // GitHub configures a token. The primary action label must diverge by kind.
  const channel = renderExtensionCard({
    package_ref: { id: "slack" },
    kind: "channel",
    onboarding_state: "setup_required",
    display_name: "Slack",
  });
  assert.equal(
    renderedContainsValue(channel, "connect"),
    true,
    "unconnected channel should offer Connect, not Configure",
  );
  assert.equal(renderedContainsValue(channel, "configure"), false);

  const credential = renderExtensionCard({
    package_ref: { id: "github" },
    kind: "mcp_server",
    onboarding_state: "setup_required",
    display_name: "GitHub",
  });
  assert.equal(
    renderedContainsValue(credential, "configure"),
    true,
    "credential extension should keep Configure",
  );
  assert.equal(renderedContainsValue(credential, "connect"), false);
});

test("active package with missing auth renders auth needed setup state", () => {
  const rendered = renderExtensionCard({
    package_ref: { kind: "extension", id: "slack" },
    kind: "wasm_tool",
    display_name: "Slack",
    active: true,
    authenticated: false,
    needs_setup: true,
    has_auth: true,
    activation_status: "active",
  });

  assert.equal(
    renderedContainsValue(rendered, "auth_required"),
    true,
    "missing Slack OAuth should show auth needed instead of active",
  );
  assert.equal(
    renderedContainsValue(rendered, "configure"),
    true,
    "missing Slack OAuth should keep the setup action available",
  );
  assert.equal(renderedContainsValue(rendered, "active"), false);
});

test("renders_channel_overflow_actions_for_setup_and_reconfigure_states", async () => {
  const runCase = (_name, assertion) => assertion();

  // --- Setup state: kind=channel, state=setup_required ---
  await runCase(
    "kind=channel in setup_required state does not duplicate primary Configure as Setup overflow",
    () => {
      const ext = {
        package_ref: { id: "telegram" },
        kind: "channel",
        onboarding_state: "setup_required",
        display_name: "Telegram",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(!ids.includes("setup"), `Expected no duplicate 'setup' overflow action, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Setup state: kind=channel, state=failed ---
  await runCase(
    "kind=channel in failed state includes Setup overflow action",
    () => {
      const ext = {
        package_ref: { id: "telegram" },
        kind: "channel",
        onboarding_state: "failed",
        display_name: "Telegram",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(ids.includes("setup"), `Expected 'setup' in overflow actions, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Setup state: kind=wasm_channel, state=setup_required ---
  await runCase(
    "kind=wasm_channel in setup_required state does not duplicate primary Configure as Setup overflow",
    () => {
      const ext = {
        package_ref: { id: "some-wasm-channel" },
        kind: "wasm_channel",
        onboarding_state: "setup_required",
        display_name: "My WASM Channel",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(!ids.includes("setup"), `Expected no duplicate 'setup' overflow action, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Setup state: kind=wasm_channel, state=failed ---
  await runCase(
    "kind=wasm_channel in failed state includes Setup overflow action",
    () => {
      const ext = {
        package_ref: { id: "some-wasm-channel" },
        kind: "wasm_channel",
        onboarding_state: "failed",
        display_name: "My WASM Channel",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(ids.includes("setup"), `Expected 'setup' in overflow actions, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Active state: kind=channel, state=active ---
  await runCase(
    "kind=channel in active state includes Reconfigure overflow action",
    () => {
      const ext = {
        package_ref: { id: "telegram" },
        kind: "channel",
        activation_status: "active",
        display_name: "Telegram",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(ids.includes("reconfigure"), `Expected 'reconfigure' in overflow actions, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Active state: kind=wasm_channel, state=active ---
  await runCase(
    "kind=wasm_channel in active state includes Reconfigure overflow action",
    () => {
      const ext = {
        package_ref: { id: "some-wasm-channel" },
        kind: "wasm_channel",
        activation_status: "active",
        display_name: "My WASM Channel",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(ids.includes("reconfigure"), `Expected 'reconfigure' in overflow actions, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Active state: kind=channel, state=ready ---
  await runCase(
    "kind=channel in ready state includes Reconfigure overflow action",
    () => {
      const ext = {
        package_ref: { id: "telegram" },
        kind: "channel",
        activation_status: "ready",
        display_name: "Telegram",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(ids.includes("reconfigure"), `Expected 'reconfigure' in overflow actions, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Active state: kind=channel, state=pairing_required ---
  await runCase(
    "kind=channel in pairing_required state includes Reconfigure overflow action",
    () => {
      const ext = {
        package_ref: { id: "telegram" },
        kind: "channel",
        activation_status: "pairing_required",
        display_name: "Telegram",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(ids.includes("reconfigure"), `Expected 'reconfigure' in overflow actions, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Non-channel kind does NOT get Setup or Reconfigure ---
  await runCase(
    "non-channel kinds do not get Setup or Reconfigure overflow actions",
    () => {
      const ext = {
        package_ref: { id: "notion" },
        kind: "mcp_server",
        onboarding_state: "setup_required",
        display_name: "Notion",
        needs_setup: true,
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      // May have a configure or remove action, but not setup/reconfigure
      if (actions !== null) {
        const ids = actions.map((a) => a.id);
        assert.ok(!ids.includes("setup"), `Expected no 'setup' action for mcp_server, got: ${JSON.stringify(ids)}`);
        assert.ok(!ids.includes("reconfigure"), `Expected no 'reconfigure' action for mcp_server, got: ${JSON.stringify(ids)}`);
      }
    },
  );

  // --- Failed-state Setup action calls onConfigure ---
  await runCase(
    "Failed-state Setup overflow action invokes onConfigure with the correct payload",
    () => {
      let configurePayload = null;
      const context = makeContext();
      vm.runInNewContext(extensionCardSourceWithInternals(), context);
      const { ExtensionCard, OverflowMenu } = context.globalThis.__testExports;

      const ext = {
        package_ref: { id: "telegram" },
        kind: "channel",
        onboarding_state: "failed",
        display_name: "Telegram",
      };
      const rendered = ExtensionCard({
        ext,
        onActivate() {},
        onConfigure(payload) { configurePayload = payload; },
        onRemove() {},
        isBusy: false,
      });

      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const setupAction = actions.find((a) => a.id === "setup");
      assert.notEqual(setupAction, undefined, "Setup action must exist");
      assert.equal(setupAction.label, "setup");
      setupAction.run();
      assert.deepEqual(configurePayload.packageRef, { id: "telegram" });
      assert.equal(configurePayload.displayName, "Telegram");
      assert.equal(configurePayload.onboardingState, "failed");
    },
  );

  // --- Reconfigure action calls onConfigure ---
  await runCase(
    "Reconfigure overflow action invokes onConfigure with the correct payload",
    () => {
      let configurePayload = null;
      const context = makeContext();
      vm.runInNewContext(extensionCardSourceWithInternals(), context);
      const { ExtensionCard, OverflowMenu } = context.globalThis.__testExports;

      const ext = {
        package_ref: { id: "telegram" },
        kind: "channel",
        activation_status: "active",
        authenticated: true,
        display_name: "Telegram",
      };
      const rendered = ExtensionCard({
        ext,
        onActivate() {},
        onConfigure(payload) { configurePayload = payload; },
        onRemove() {},
        isBusy: false,
      });

      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const reconfigureAction = actions.find((a) => a.id === "reconfigure");
      assert.notEqual(reconfigureAction, undefined, "Reconfigure action must exist");
      // A connected channel re-pairs via "Reconnect", not "Reconfigure".
      assert.equal(reconfigureAction.label, "reconnect");
      reconfigureAction.run();
      assert.deepEqual(configurePayload.packageRef, { id: "telegram" });
      assert.equal(configurePayload.displayName, "Telegram");
      assert.equal(configurePayload.activationStatus, "active");
    },
  );
});
