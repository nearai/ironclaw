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
  packageRef = { kind: "extension", id: "slack" },
  channel = undefined,
  displayName = "Slack",
  onboardingState = "pairing_required",
  authenticated = false,
  onClose = () => {},
  onSaved,
  activate = async () => {},
  translate,
  redeemPairingCode,
} = {}) {
  const calls = [];
  const invalidations = [];
  let mutationConfig = null;
  const redeem =
    redeemPairingCode ||
    (async (channel, code) => {
      calls.push(["redeem", channel, code]);
      return { success: true };
    });
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
    useT: () => translate || ((key) => key),
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
    extensionLifecycleState: (extension) =>
      extension?.onboarding_state ||
      extension?.onboardingState ||
      extension?.activation_status ||
      extension?.activationStatus ||
      (extension?.active ? "active" : "installed"),
    setupReadyForActivation: () => false,
    isChannelExtensionKind: (k) => k === "channel" || k === "wasm_channel",
    redeemPairingCode: redeem,
    activateExtension: async (ref) => {
      calls.push(["activate", ref]);
      await activate();
    },
    globalThis: {},
  };

  vm.runInNewContext(configureModalSourceForTest(), context);
  const rendered = context.globalThis.__testExports.ConfigureModal({
    extension: {
      packageRef,
      displayName,
      kind,
      channel,
      onboarding_state: onboardingState,
      authenticated,
    },
    onClose,
    onSaved,
  });
  return { calls, invalidations, mutationConfig, rendered };
}

test("ConfigureModal renders the pairing panel for a channel extension", () => {
  const { rendered, mutationConfig } = renderModal({
    kind: "channel",
    onboardingState: "pairing_required",
  });
  assert.ok(mutationConfig, "a pairing mutation must be configured");
  const body = JSON.stringify(rendered);
  assert.match(body, /pairing\.slackPlaceholder/);
  assert.match(body, /pairing\.slackInstructions/);
});

test("ConfigureModal does not render the pairing panel for a non-channel extension", () => {
  const { rendered } = renderModal({ kind: "mcp_server" });
  const body = JSON.stringify(rendered);
  assert.doesNotMatch(body, /pairing\.slackPlaceholder/);
  assert.doesNotMatch(body, /pairing\.slackInstructions/);
});

test("ConfigureModal routes unconnected setup-required channels to the Connect panel", () => {
  // A just-installed channel is in `setup_required` but still needs the user to
  // connect (pair) — it must land on the Connect panel, never "no config".
  const { rendered } = renderModal({
    kind: "channel",
    onboardingState: "setup_required",
    authenticated: false,
  });

  const body = JSON.stringify(rendered);
  assert.match(body, /pairing\.slackPlaceholder/);
  assert.match(body, /pairing\.slackInstructions/);
  assert.doesNotMatch(body, /extensions\.noConfigRequired/);
});

test("ConfigureModal renders the Connect panel for the freshly-installed ground-truth state", () => {
  // Exact ground-truth of a freshly-installed Slack channel:
  // kind=channel, onboarding_state=setup_required, authenticated=false.
  const { rendered } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "slack" },
    displayName: "Slack",
    onboardingState: "setup_required",
    authenticated: false,
  });

  const body = JSON.stringify(rendered);
  assert.match(body, /pairing\.slackPlaceholder/);
  assert.match(body, /pairing\.slackInstructions/);
  assert.doesNotMatch(body, /extensions\.noConfigRequired/);
});

test("ConfigureModal localizes channel pairing copy", () => {
  const { rendered } = renderModal({
    kind: "channel",
    onboardingState: "pairing_required",
    translate: (key) =>
      ({
        "extensions.configureName": "Configure {name}",
        "pairing.slackInstructions": "Localized Slack pairing instructions",
        "pairing.slackPlaceholder": "Localized Slack pairing placeholder",
        "pairing.connect": "Localized connect",
        "common.saving": "Localized saving",
        "pairing.slackError": "Localized Slack pairing error",
      })[key] || key,
  });
  const body = JSON.stringify(rendered);

  assert.match(body, /Localized Slack pairing instructions/);
  assert.match(body, /Localized Slack pairing placeholder/);
  assert.match(body, /Localized connect/);
  assert.doesNotMatch(body, /Open Slack and message/);
  assert.doesNotMatch(body, /Enter pairing code/);
});

test("ConfigureModal pairing redeems then activates, invalidates queries, and closes", async () => {
  let closed = false;
  const { calls, invalidations, mutationConfig } = renderModal({
    kind: "channel",
    onboardingState: "pairing_required",
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

test("ConfigureModal pairing redeems by channel slug and activates package id", async () => {
  const { calls, invalidations, mutationConfig } = renderModal({
    kind: "channel",
    onboardingState: "pairing_required",
    packageRef: { kind: "extension", id: "slack-host-package" },
    channel: "slack",
  });

  await mutationConfig.mutationFn("A1B2C3");
  mutationConfig.onSuccess();

  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    ["redeem", "slack", "A1B2C3"],
    ["activate", { id: "slack-host-package" }],
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["extensions"],
    ["connectable-channels"],
    ["pairing", "slack"],
  ]);
});

test("ConfigureModal treats post-redeem activation failure as best-effort", async () => {
  const { calls, mutationConfig } = renderModal({
    kind: "channel",
    onboardingState: "pairing_required",
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
