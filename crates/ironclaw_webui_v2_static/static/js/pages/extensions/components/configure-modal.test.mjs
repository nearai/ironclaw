import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import { rememberChannelConnectionWaiter } from "../../../lib/channel-connection-events.js";
import { redeemPairingCode as realRedeemPairingCode } from "../lib/pairing-api.js";

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
  assert.doesNotMatch(JSON.stringify(rendered), /Enter pairing code/);
});

test("ConfigureModal does not route setup-required channels to the pairing panel", () => {
  const { rendered } = renderModal({
    kind: "channel",
    onboardingState: "setup_required",
  });

  assert.doesNotMatch(JSON.stringify(rendered), /pairing\.slackPlaceholder/);
  assert.doesNotMatch(JSON.stringify(rendered), /pairing\.slackInstructions/);
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

test("ConfigureModal pairing through the real API waits for blocked chats to resume", async (t) => {
  const originalWindow = globalThis.window;
  const originalFetch = globalThis.fetch;
  const originalSessionStorage = globalThis.sessionStorage;
  t.after(() => {
    globalThis.window = originalWindow;
    globalThis.fetch = originalFetch;
    globalThis.sessionStorage = originalSessionStorage;
  });

  const storage = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => (storage.has(key) ? storage.get(key) : null),
      setItem: (key, value) => storage.set(key, String(value)),
      removeItem: (key) => storage.delete(key),
    },
    addEventListener: () => {},
    removeEventListener: () => {},
  };
  globalThis.sessionStorage = {
    getItem: () => "token-1",
    setItem: () => {},
    removeItem: () => {},
  };
  rememberChannelConnectionWaiter({
    channel: "slack",
    threadId: "thread-waiting",
    sourceMessageId: "tool-1",
  });

  let releaseResume;
  const resumeGate = new Promise((resolve) => {
    releaseResume = resolve;
  });
  const fetches = [];
  globalThis.fetch = async (path, options) => {
    fetches.push({ path, options });
    if (path === "/api/webchat/v2/extensions/pairing/redeem") {
      return new Response(
        JSON.stringify({ provider: "slack", provider_user_id: "install-alpha:U123" }),
        {
          status: 200,
          headers: { "content-type": "application/json" },
        },
      );
    }
    if (path === "/api/webchat/v2/threads/thread-waiting/messages") {
      await resumeGate;
      return new Response(JSON.stringify({ run_id: "run-1" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    throw new Error(`unexpected fetch: ${path}`);
  };

  const { mutationConfig } = renderModal({
    kind: "channel",
    redeemPairingCode: realRedeemPairingCode,
  });
  let settled = false;
  const mutationPromise = mutationConfig.mutationFn("A1B2C3").then((value) => {
    settled = true;
    return value;
  });
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(fetches.length, 2);
  assert.equal(fetches[0].path, "/api/webchat/v2/extensions/pairing/redeem");
  assert.equal(fetches[1].path, "/api/webchat/v2/threads/thread-waiting/messages");
  assert.deepEqual(JSON.parse(fetches[0].options.body), {
    channel: "slack",
    code: "A1B2C3",
  });
  assert.equal(
    JSON.parse(fetches[1].options.body).content,
    "Slack is connected. Continue the previous request.",
  );
  assert.equal(settled, false, "the modal mutation must wait for resume to finish");

  releaseResume();
  assert.deepEqual(await mutationPromise, {
    success: true,
    provider: "slack",
    provider_user_id: "install-alpha:U123",
  });
  assert.equal(settled, true);
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
