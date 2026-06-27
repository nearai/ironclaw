import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function configureModalSourceForTest() {
  const source = readFileSync(new URL("./configure-modal.js", import.meta.url), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ConfigureModal };`;
}

function renderModal({
  kind = "channel",
  onClose = () => {},
  onSaved,
  activate = async () => {},
} = {}) {
  const calls = [];
  const invalidations = [];
  let mutationConfig = null;
  const context = {
    useMutation: (config) => {
      mutationConfig = config;
      return { isPending: false, isError: false, error: null, mutate() {} };
    },
    useQueryClient: () => ({
      invalidateQueries: ({ queryKey }) => invalidations.push(queryKey),
    }),
    Button() {},
    Icon() {},
    console: { error() {} },
    React: {
      useState: (initial) => [
        typeof initial === "function" ? initial() : initial,
        () => {},
      ],
      useCallback: (fn) => fn,
      useEffect: () => {},
      useRef: (value) => ({ current: value }),
    },
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    useT: () => (key) => key,
    useExtensionSetup: () => ({
      secrets: [],
      fields: [],
      onboarding: null,
      isLoading: false,
      error: null,
    }),
    useOauthSetup: () => ({ mutate() {}, isPending: false, error: null }),
    useSetupSubmit: () => ({ mutate() {}, isPending: false, error: null }),
    extensionIsActive: () => false,
    setupReadyForActivation: () => false,
    isChannelExtensionKind: (k) => k === "channel" || k === "wasm_channel",
    redeemPairingCode: async (channel, code) => {
      calls.push(["redeem", channel, code]);
      return { success: true };
    },
    activateExtension: async (ref) => {
      calls.push(["activate", ref]);
      await activate();
    },
    globalThis: {},
  };

  vm.runInNewContext(configureModalSourceForTest(), context);
  const rendered = context.globalThis.__testExports.ConfigureModal({
    extension: {
      packageRef: { kind: "extension", id: "slack" },
      displayName: "Slack",
      kind,
    },
    onClose,
    onSaved,
  });
  return { calls, invalidations, mutationConfig, rendered };
}

test("ConfigureModal renders the pairing panel for a channel extension", () => {
  const { rendered, mutationConfig } = renderModal({ kind: "channel" });
  assert.ok(mutationConfig, "a pairing mutation must be configured");
  const body = JSON.stringify(rendered);
  assert.match(body, /Enter pairing code/);
  assert.match(body, /Open Slack and message the IronClaw Reborn app/);
  assert.match(body, /never sent to the model/);
});

test("ConfigureModal does not render the pairing panel for a non-channel extension", () => {
  const { rendered } = renderModal({ kind: "mcp_server" });
  assert.doesNotMatch(JSON.stringify(rendered), /Enter pairing code/);
});

test("ConfigureModal pairing redeems then activates, invalidates queries, and closes", async () => {
  let closed = false;
  const { calls, invalidations, mutationConfig } = renderModal({
    kind: "channel",
    onClose: () => {
      closed = true;
    },
  });

  const result = await mutationConfig.mutationFn("A1B2C3");
  mutationConfig.onSuccess();

  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    ["redeem", "slack", "A1B2C3"],
    ["activate", { id: "slack" }],
  ]);
  assert.deepEqual(result, { success: true });
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["extensions"],
    ["connectable-channels"],
    ["pairing", "slack"],
  ]);
  assert.equal(closed, true);
});

test("ConfigureModal treats post-redeem activation failure as best-effort", async () => {
  const { calls, mutationConfig } = renderModal({
    kind: "channel",
    activate: async () => {
      throw new Error("activation boom");
    },
  });

  // The mutation must resolve with the successful redemption even though the
  // follow-up activation threw — a connected account is not surfaced as a
  // pairing failure.
  const result = await mutationConfig.mutationFn("A1B2C3");
  assert.deepEqual(result, { success: true });
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    ["redeem", "slack", "A1B2C3"],
    ["activate", { id: "slack" }],
  ]);
});
