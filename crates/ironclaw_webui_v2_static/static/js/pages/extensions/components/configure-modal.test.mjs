import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// ---------------------------------------------------------------------------
// Source munging — strip ES module imports, rewrite exports, inject test shim.
// Mirrors the approach in extension-card.test.mjs. `SharedCredentialSection` is
// stubbed as an identity marker so we can find it in the rendered tree and read
// the `defaultHandle` prop passed to it.
// ---------------------------------------------------------------------------

function stripImports(source) {
  const lines = source.split("\n");
  const out = [];
  let inBlockImport = false;
  for (const line of lines) {
    if (inBlockImport) {
      if (/^\s*\}/.test(line) && /from\s+["']/.test(line)) {
        inBlockImport = false;
      }
      continue;
    }
    if (line.startsWith("import ")) {
      if (line.includes("{") && !line.includes("}")) {
        inBlockImport = true;
      }
      continue;
    }
    out.push(line.replace(/^export function /, "function "));
  }
  return out.join("\n");
}

function configureModalSourceForTest() {
  const source = readFileSync(new URL("./configure-modal.js", import.meta.url), "utf8");
  return stripImports(source) + "\nglobalThis.__testExports = { ConfigureModal, SharedCredentialSection };";
}

function makeContext({ secrets = [], fields = [] } = {}) {
  const React = {
    useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    useCallback: (fn) => fn,
    useEffect: () => {},
    useId: () => "id",
  };
  function html(strings, ...values) {
    return { strings: Array.from(strings), values };
  }
  function useT() {
    return (key) => key;
  }
  function Button() {}
  function Icon() {}
  // Identity marker for the shared-credential section so the tree walker can
  // find it and read its props.
  function SharedCredentialSection() {}
  function useExtensionSetup() {
    return { secrets, fields, onboarding: null, isLoading: false, error: null };
  }
  function useOauthSetup() {
    return { mutate: () => {}, isPending: false, error: null };
  }
  function useSetupSubmit() {
    return { mutate: () => {}, isPending: false, error: null };
  }
  function extensionIsActive() {
    return false;
  }
  function setupReadyForActivation() {
    return false;
  }

  return {
    globalThis: {},
    React,
    html,
    useT,
    Button,
    Icon,
    SharedCredentialSection,
    useExtensionSetup,
    useOauthSetup,
    useSetupSubmit,
    extensionIsActive,
    setupReadyForActivation,
  };
}

function renderConfigureModal({ extension, isAdmin, setup }) {
  const context = makeContext(setup);
  vm.runInNewContext(configureModalSourceForTest(), context);
  const { ConfigureModal, SharedCredentialSection } = context.globalThis.__testExports;
  const rendered = ConfigureModal({
    extension,
    isAdmin,
    onActivate() {},
    onClose() {},
    onSaved() {},
  });
  return { rendered, SharedCredentialSection };
}

// Walk the rendered tree; collect every node that is an html`` call whose
// values[0] is the SharedCredentialSection marker.
function findSharedSections(node, marker, acc = []) {
  if (!node || typeof node !== "object") return acc;
  if (Array.isArray(node)) {
    for (const v of node) findSharedSections(v, marker, acc);
    return acc;
  }
  if (Array.isArray(node.values)) {
    if (node.values[0] === marker) acc.push(node);
    for (const v of node.values) findSharedSections(v, marker, acc);
  }
  return acc;
}

// Extract the `defaultHandle=${...}` prop from an html`` call for the marker.
// The template `<${SharedCredentialSection} key=... defaultHandle=${cred.handle} />`
// puts the marker at values[0], the key at values[1], the handle at values[2].
function defaultHandleOf(node) {
  return node.values[2];
}

// ---------------------------------------------------------------------------
// Tests (#5459)
// ---------------------------------------------------------------------------

test("SharedCredentialSection does not render for an extension without shared credentials", () => {
  // web-access declares no shared credential; even an admin must not see the form.
  const { rendered, SharedCredentialSection } = renderConfigureModal({
    extension: { displayName: "Web Access", packageRef: { id: "web-access" }, sharedCredentials: [] },
    isAdmin: true,
    setup: { secrets: [], fields: [] },
  });
  const sections = findSharedSections(rendered, SharedCredentialSection);
  assert.equal(sections.length, 0, "no SharedCredentialSection should render without declared shared credentials");
});

test("SharedCredentialSection renders for an admin when the extension declares a shared credential", () => {
  const { rendered, SharedCredentialSection } = renderConfigureModal({
    extension: {
      displayName: "Market Data",
      packageRef: { id: "market-data" },
      sharedCredentials: [{ handle: "market_data_api_key", required: true }],
    },
    isAdmin: true,
    setup: { secrets: [], fields: [] },
  });
  const sections = findSharedSections(rendered, SharedCredentialSection);
  assert.equal(sections.length, 1, "exactly one SharedCredentialSection should render");
  assert.equal(
    defaultHandleOf(sections[0]),
    "market_data_api_key",
    "the declared manifest handle is passed as the fixed defaultHandle"
  );
});

test("SharedCredentialSection is hidden from non-admins even when a shared credential is declared", () => {
  const { rendered, SharedCredentialSection } = renderConfigureModal({
    extension: {
      displayName: "Market Data",
      packageRef: { id: "market-data" },
      sharedCredentials: [{ handle: "market_data_api_key", required: true }],
    },
    isAdmin: false,
    setup: { secrets: [], fields: [] },
  });
  const sections = findSharedSections(rendered, SharedCredentialSection);
  assert.equal(sections.length, 0, "non-admins never see the shared-credential form");
});
