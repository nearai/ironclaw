// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import {
  RUNTIME_LABELS,
  STATE_LABELS,
  STATE_TONES,
  hasAuthSurface,
  hasChannelSurface,
  primaryAuthAccount,
  authAccountNeedsReconnect,
  authAccountReasonLabelKey,
} from "../lib/extensions-schema";
import { extensionLifecycleState, primaryExtensionAction } from "../lib/extension-actions";

const channelSurfaces = [{ kind: "channel", inbound: true, outbound: true }];
const toolSurfaces = [{ kind: "tool" }];

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
 *   - RUNTIME_LABELS, STATE_TONES, STATE_LABELS, hasChannelSurface,
 *     primaryAuthAccount, authAccountNeedsReconnect, authAccountReasonLabelKey
 *     — the REAL exports of extensions-schema (imported above), so the card
 *     is exercised against the production surface/runtime/auth-account model
 *     with no drift risk
 *   - primaryExtensionAction — the REAL export of extension-actions
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

  return {
    globalThis: {},
    React,
    useT,
    Badge,
    Button,
    Icon,
    hasAuthSurface,
    hasChannelSurface,
    primaryAuthAccount,
    authAccountNeedsReconnect,
    authAccountReasonLabelKey,
    RUNTIME_LABELS,
    STATE_TONES,
    STATE_LABELS,
    extensionLifecycleState,
    primaryExtensionAction,
  };
}

/**
 * Render ExtensionCard with the given ext prop and return the rendered tree.
 * onConfigure / onRemove are no-op stubs.
 */
function renderExtensionCard(ext) {
  const context = makeContext();
  vm.runInNewContext(extensionCardSourceForTest(), context);
  const { ExtensionCard } = context.globalThis.__testExports;
  return ExtensionCard({
    ext,
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
    runtime: "wasm",
    surfaces: channelSurfaces,
    display_name: "Telegram",
  });
  const cardClass = rendered.values[0];

  assert.ok(
    cardClass.includes("self-start"),
    "CARD should align itself to the top of its grid area",
  );
  assert.ok(!cardClass.includes("h-full"), "CARD must not stretch to grid row height");
});

test("setup-needed cards never expose a separate Activate action", () => {
  const channel = renderExtensionCard({
    package_ref: { id: "slack" },
    runtime: "first_party",
    surfaces: channelSurfaces,
    installation_state: "setup_needed",
    display_name: "Slack",
  });
  assert.equal(
    renderedContainsValue(channel, "Activate"),
    false,
    "Slack-style channels should use configure/setup instead of generic activation",
  );

  const mcp = renderExtensionCard({
    package_ref: { id: "github" },
    runtime: "mcp",
    surfaces: toolSurfaces,
    installation_state: "setup_needed",
    display_name: "GitHub",
  });
  assert.equal(
    renderedContainsValue(mcp, "activate"),
    false,
    "internal activation must never surface as a second user action",
  );
});

test("setup-needed cards render the shared yellow warning status pill", () => {
  const rendered = renderExtensionCard({
    package_ref: { id: "slack" },
    runtime: "first_party",
    surfaces: channelSurfaces,
    installation_state: "setup_needed",
    display_name: "Slack",
  });

  assert.equal(
    renderedContainsValue(rendered, "warning"),
    true,
    "the ExtensionCard caller must pass the warning tone to Badge",
  );
});

test("setup-required primary action reads Connect for a channel and Configure for a credential extension", () => {
  // A freshly-installed channel connects (pairs); a credential extension like
  // GitHub configures a token. The primary action label must diverge by the
  // extension's declared surfaces.
  const channel = renderExtensionCard({
    package_ref: { id: "slack" },
    runtime: "first_party",
    surfaces: channelSurfaces,
    installation_state: "setup_needed",
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
    runtime: "mcp",
    surfaces: toolSurfaces,
    installation_state: "setup_needed",
    display_name: "GitHub",
  });
  assert.equal(
    renderedContainsValue(credential, "configure"),
    true,
    "credential extension should keep Configure",
  );
  assert.equal(renderedContainsValue(credential, "connect"), false);
});

test("expired channel account renders the Reconnect (expired) affordance and expiry notice (G4)", () => {
  // A channel whose §6.3 account state is `expired` must not read as a
  // first-time Connect: the affordance becomes the distinct expired-reconnect
  // label and the card surfaces an expiry notice derived from the account state.
  const rendered = renderExtensionCard({
    package_ref: { id: "acme" },
    runtime: "first_party",
    surfaces: channelSurfaces,
    installation_state: "setup_needed",
    display_name: "Acme",
    auth_accounts: [
      {
        vendor: "acme",
        accounts: [
          { account_id: "acme", state: "expired", last_error: "refresh_failed", is_default: true },
        ],
      },
    ],
  });
  assert.equal(
    renderedContainsValue(rendered, "reconnectExpired"),
    true,
    "an expired account must offer the Reconnect (expired) affordance",
  );
  assert.equal(
    renderedContainsValue(rendered, "accountExpired"),
    true,
    "an expired account must render an expiry notice",
  );
  // It is not a fresh Connect.
  assert.equal(renderedContainsValue(rendered, "connect"), false);
});

test("healthy connected channel account shows Connect/Reconnect but no expiry affordance (G4)", () => {
  const { rendered, OverflowMenu } = renderExtensionCardWithInternals({
    package_ref: { id: "acme" },
    runtime: "first_party",
    surfaces: channelSurfaces,
    installation_state: "active",
    display_name: "Acme",
    auth_accounts: [
      { vendor: "acme", accounts: [{ account_id: "acme", state: "connected", is_default: true }] },
    ],
  });
  assert.equal(
    renderedContainsValue(rendered, "reconnectExpired"),
    false,
    "a connected account must not use the expired affordance",
  );
  assert.equal(
    renderedContainsValue(rendered, "accountExpired"),
    false,
    "a connected account must not render an expiry notice",
  );
  assert.equal(
    extractOverflowActions(rendered, OverflowMenu).some(
      (action) => action.label === "reconnect"
    ),
    true,
    "a connected active channel keeps the Reconnect affordance",
  );
});

test("failed extension renders its activation_error as a danger reason banner", () => {
  // Internal failures remain attached to the public setup-needed state. The
  // card must surface the redacted reason regardless of runtime/surfaces.
  const rendered = renderExtensionCard({
    package_ref: { id: "acme" },
    runtime: "first_party",
    display_name: "Acme",
    installation_state: "setup_needed",
    activation_error: "The vendor webhook returned a 500.",
  });
  assert.equal(
    renderedContainsValue(rendered, "The vendor webhook returned a 500."),
    true,
    "a failed extension must render its redacted activation_error reason",
  );
});

test("disconnected auth accounts render a distinct reason per last_error, not a generic expiry notice (G4)", () => {
  // A revoked grant is disconnected-with-a-reason, not the `expired` state —
  // it must still offer Reconnect with its own copy, not the refresh_failed
  // expiry notice or (retired) `revoking` copy.
  const revoked = renderExtensionCard({
    package_ref: { id: "acme" },
    runtime: "first_party",
    surfaces: channelSurfaces,
    installation_state: "setup_needed",
    display_name: "Acme",
    auth_accounts: [
      {
        vendor: "acme",
        accounts: [
          { account_id: "acme", state: "disconnected", last_error: "grant_revoked", is_default: true },
        ],
      },
    ],
  });
  assert.equal(
    renderedContainsValue(revoked, "accountRevoked"),
    true,
    "a revoked grant must render its own reason",
  );
  assert.equal(renderedContainsValue(revoked, "accountExpired"), false);
  assert.equal(
    renderedContainsValue(revoked, "reconnectExpired"),
    true,
    "a revoked account still offers the reconnect affordance",
  );

  const missingCredential = renderExtensionCard({
    package_ref: { id: "acme" },
    runtime: "first_party",
    surfaces: channelSurfaces,
    installation_state: "setup_needed",
    display_name: "Acme",
    auth_accounts: [
      {
        vendor: "acme",
        accounts: [
          {
            account_id: "acme",
            state: "disconnected",
            last_error: "credential_missing",
            is_default: true,
          },
        ],
      },
    ],
  });
  assert.equal(
    renderedContainsValue(missingCredential, "accountCredentialMissing"),
    true,
    "a missing credential must render its own reason",
  );
  assert.equal(renderedContainsValue(missingCredential, "accountRevoked"), false);
  assert.equal(renderedContainsValue(missingCredential, "accountExpired"), false);

  // A fresh, never-connected account (disconnected, no last_error) stays a
  // plain first-time Connect with no reason banner at all.
  const fresh = renderExtensionCard({
    package_ref: { id: "acme" },
    runtime: "first_party",
    surfaces: channelSurfaces,
    installation_state: "setup_needed",
    display_name: "Acme",
    auth_accounts: [
      { vendor: "acme", accounts: [{ account_id: "acme", state: "disconnected", is_default: true }] },
    ],
  });
  assert.equal(renderedContainsValue(fresh, "accountRevoked"), false);
  assert.equal(renderedContainsValue(fresh, "accountCredentialMissing"), false);
  assert.equal(renderedContainsValue(fresh, "accountExpired"), false);
  assert.equal(
    renderedContainsValue(fresh, "reconnectExpired"),
    false,
    "a fresh never-connected account is not a reconnect",
  );
  assert.equal(renderedContainsValue(fresh, "connect"), true);
});

test("setup-needed package renders the canonical setup state", () => {
  const rendered = renderExtensionCard({
    package_ref: { kind: "extension", id: "slack" },
    runtime: "first_party",
    surfaces: channelSurfaces,
    display_name: "Slack",
    installation_state: "setup_needed",
  });

  assert.equal(
    renderedContainsValue(rendered, "setup_needed"),
    true,
    "missing Slack OAuth should show auth needed instead of active",
  );
  assert.equal(
    renderedContainsValue(rendered, "connect"),
    true,
    "a setup-needed channel should keep its connect action available",
  );
  assert.equal(renderedContainsValue(rendered, "active"), false);
});

test("renders_channel_overflow_actions_for_setup_and_reconfigure_states", async () => {
  const runCase = (_name, assertion) => assertion();

  // --- Setup state: first_party runtime + channel surface, state=setup_required ---
  await runCase(
    "first_party channel surface in setup_required state does not duplicate primary Configure as Setup overflow",
    () => {
      const ext = {
        package_ref: { id: "telegram" },
        runtime: "first_party",
        surfaces: channelSurfaces,
        installation_state: "setup_needed",
        display_name: "Telegram",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(!ids.includes("setup"), `Expected no duplicate 'setup' overflow action, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Internal failure projects to the same public setup_needed state ---
  await runCase(
    "first_party setup_needed channel does not duplicate its primary Configure action",
    () => {
      const ext = {
        package_ref: { id: "telegram" },
        runtime: "first_party",
        surfaces: channelSurfaces,
        installation_state: "setup_needed",
        display_name: "Telegram",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(!ids.includes("setup"), `Expected no duplicate 'setup' action, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Setup state: wasm runtime + channel surface, state=setup_required ---
  await runCase(
    "wasm channel surface in setup_required state does not duplicate primary Configure as Setup overflow",
    () => {
      const ext = {
        package_ref: { id: "some-wasm-channel" },
        runtime: "wasm",
        surfaces: channelSurfaces,
        installation_state: "setup_needed",
        display_name: "My WASM Channel",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(!ids.includes("setup"), `Expected no duplicate 'setup' overflow action, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Same projection for a wasm-backed channel ---
  await runCase(
    "wasm setup_needed channel does not duplicate its primary Configure action",
    () => {
      const ext = {
        package_ref: { id: "some-wasm-channel" },
        runtime: "wasm",
        surfaces: channelSurfaces,
        installation_state: "setup_needed",
        display_name: "My WASM Channel",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(!ids.includes("setup"), `Expected no duplicate 'setup' action, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Active state: first_party runtime + channel surface, state=active ---
  await runCase(
    "first_party channel surface in active state includes Reconfigure overflow action",
    () => {
      const ext = {
        package_ref: { id: "telegram" },
        runtime: "first_party",
        surfaces: channelSurfaces,
        installation_state: "active",
        display_name: "Telegram",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(ids.includes("reconfigure"), `Expected 'reconfigure' in overflow actions, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Active state: wasm runtime + channel surface, state=active ---
  await runCase(
    "wasm channel surface in active state includes Reconfigure overflow action",
    () => {
      const ext = {
        package_ref: { id: "some-wasm-channel" },
        runtime: "wasm",
        surfaces: channelSurfaces,
        installation_state: "active",
        display_name: "My WASM Channel",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(ids.includes("reconfigure"), `Expected 'reconfigure' in overflow actions, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Legacy ready input does not invent a fourth public state ---
  await runCase(
    "legacy ready input is treated as setup_needed, not active",
    () => {
      const ext = {
        package_ref: { id: "telegram" },
        runtime: "first_party",
        surfaces: channelSurfaces,
        installation_state: "ready",
        display_name: "Telegram",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(!ids.includes("reconfigure"), `Expected no active-only 'reconfigure' action, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- Legacy pairing_required input also projects to setup_needed ---
  await runCase(
    "legacy pairing_required input does not render active-only Reconfigure",
    () => {
      const ext = {
        package_ref: { id: "telegram" },
        runtime: "first_party",
        surfaces: channelSurfaces,
        installation_state: "pairing_required",
        display_name: "Telegram",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      const ids = actions.map((a) => a.id);
      assert.ok(!ids.includes("reconfigure"), `Expected no active-only 'reconfigure' action, got: ${JSON.stringify(ids)}`);
    },
  );

  // --- No channel surface does NOT get Setup or Reconfigure ---
  await runCase(
    "extensions without a channel surface do not get Setup or Reconfigure overflow actions",
    () => {
      const ext = {
        package_ref: { id: "notion" },
        runtime: "mcp",
        surfaces: toolSurfaces,
        installation_state: "setup_needed",
        display_name: "Notion",
      };
      const { rendered, OverflowMenu } = renderExtensionCardWithInternals(ext);
      const actions = extractOverflowActions(rendered, OverflowMenu);
      // May have a configure or remove action, but not setup/reconfigure
      if (actions !== null) {
        const ids = actions.map((a) => a.id);
        assert.ok(!ids.includes("setup"), `Expected no 'setup' action for a tool-surface MCP extension, got: ${JSON.stringify(ids)}`);
        assert.ok(!ids.includes("reconfigure"), `Expected no 'reconfigure' action for a tool-surface MCP extension, got: ${JSON.stringify(ids)}`);
      }
    },
  );

  // --- Internal failure projects to setup_needed with one primary configure
  // action, never a duplicate overflow Setup action. ---
  await runCase(
    "setup_needed failure projection has no duplicate Setup overflow action",
    () => {
      let configurePayload = null;
      const context = makeContext();
      vm.runInNewContext(extensionCardSourceWithInternals(), context);
      const { ExtensionCard, OverflowMenu } = context.globalThis.__testExports;

      const ext = {
        package_ref: { id: "telegram" },
        runtime: "first_party",
        surfaces: channelSurfaces,
        installation_state: "setup_needed",
        display_name: "Telegram",
      };
      const rendered = ExtensionCard({
        ext,
        onConfigure(payload) { configurePayload = payload; },
        onRemove() {},
        isBusy: false,
      });

      const actions = extractOverflowActions(rendered, OverflowMenu);
      assert.notEqual(actions, null, "OverflowMenu should be present");
      assert.equal(actions.some((action) => action.id === "setup"), false);
      assert.equal(configurePayload, null);
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
        runtime: "first_party",
        surfaces: channelSurfaces,
        installation_state: "active",
        display_name: "Telegram",
        auth_accounts: [
          {
            vendor: "telegram",
            accounts: [
              {
                account_id: "telegram",
                state: "connected",
                is_default: true,
              },
            ],
          },
        ],
      };
      const rendered = ExtensionCard({
        ext,
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
      assert.equal(configurePayload.installation_state, "active");
    },
  );
});
