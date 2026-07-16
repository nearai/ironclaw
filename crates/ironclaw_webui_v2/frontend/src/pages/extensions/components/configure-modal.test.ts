// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import { rememberChannelConnectionWaiter } from "../../../lib/channel-connection-events";
import { redeemPairingCode as realRedeemPairingCode } from "../lib/pairing-api";

function configureModalSourceForTest() {
  const source = readFileSync(new URL("./configure-modal.tsx", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { ConfigureModal, ModalShell };`;
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
  setupResult,
  setupReady = false,
  oauthMutationState = {},
  runEffects = false,
  blockPopup = false,
} = {}) {
  const calls = [];
  const invalidations = [];
  const stateSets = [];
  const oauthCalls = [];
  const oauthSetupArgs = [];
  const openedPopups = [];
  const notifications = [];
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
        (next) => {
          stateSets.push(next);
        },
      ],
      useCallback: (fn) => fn,
      useEffect: (fn) => {
        if (runEffects) fn();
      },
      useRef: (value) => ({ current: value }),
      useId: () => "configure-modal-title",
    },
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    useT: () => translate || ((key) => key),
    useExtensionSetup: () =>
      setupResult || {
        secrets: [],
        fields: [],
        onboarding: null,
        isLoading: false,
        error: null,
      },
    useOauthSetup: (...args) => {
      oauthSetupArgs.push(args);
      return {
        mutate(payload) {
          oauthCalls.push(payload);
        },
        isPending: false,
        isAuthorizing: false,
        error: null,
        ...oauthMutationState,
      };
    },
    useSetupSubmit: () => ({ mutate() {}, isPending: false, error: null }),
    extensionIsActive: () => false,
    extensionLifecycleState: (extension) =>
      extension?.onboarding_state ||
      extension?.onboardingState ||
      extension?.activation_status ||
      extension?.activationStatus ||
      (extension?.active ? "active" : "installed"),
    setupReadyForActivation: () => setupReady,
    isChannelExtensionKind: (k) => k === "channel" || k === "wasm_channel",
    redeemPairingCode: redeem,
    notifyChannelConnected: async (payload) => {
      notifications.push(payload);
    },
    activateExtension: async (ref) => {
      calls.push(["activate", ref]);
      await activate();
    },
    window: {
      open: (url, target, features) => {
        if (blockPopup) {
          openedPopups.push({ url, target, features, popup: null });
          return null;
        }
        const popup = { closed: false, location: { href: url }, opener: "test-opener" };
        openedPopups.push({ url, target, features, popup });
        return popup;
      },
      addEventListener() {},
      removeEventListener() {},
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
  return {
    calls,
    context,
    invalidations,
    mutationConfig,
    notifications,
    oauthCalls,
    oauthSetupArgs,
    openedPopups,
    rendered,
    stateSets,
  };
}

function renderFirstComponent(rendered, component, props = {}) {
  if (!rendered || !Array.isArray(rendered.values)) return null;
  if (rendered.values[0] === component) {
    return component({
      onClose: rendered.values[1],
      title: rendered.values[2],
      ...props,
    });
  }
  for (const value of rendered.values) {
    const child = renderFirstComponent(value, component, props);
    if (child) return child;
  }
  return null;
}

// Walks the rendered html-template tree and returns the first captured
// function value whose source contains the given marker (e.g. the
// `() => handleOauth(secret)` click closure). Lets tests drive captured
// handlers without a DOM.
function findHandler(node, bodyMarker, seen = new Set()) {
  if (typeof node === "function") {
    return String(node).includes(bodyMarker) ? node : null;
  }
  if (!node || typeof node !== "object" || seen.has(node)) return null;
  seen.add(node);
  const children = Array.isArray(node) ? node : Object.values(node);
  for (const child of children) {
    const found = findHandler(child, bodyMarker, seen);
    if (found) return found;
  }
  return null;
}

test("ConfigureModal renders the code-entry panel for a channel extension that uses manual setup", () => {
  const { rendered, mutationConfig } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "telegram" },
    channel: "telegram",
    displayName: "Telegram",
    onboardingState: "pairing_required",
  });
  assert.ok(mutationConfig, "a pairing mutation must be configured");
  const body = JSON.stringify(rendered);
  assert.match(body, /pairing\.placeholder/);
  assert.match(body, /pairing\.instructions/);
});

test("ConfigureModal renders Slack OAuth without opening the popup automatically", () => {
  const slackOauthSecret = {
    name: "slack_personal_oauth",
    provider: "slack_personal",
    prompt: "Slack credential",
    provided: false,
    setup: {
      kind: "oauth",
      account_label: "slack slack_personal",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  const { rendered, oauthCalls, openedPopups } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    onboardingState: "pairing_required",
    setupResult: {
      secrets: [slackOauthSecret],
      fields: [],
      onboarding: {
        credential_instructions: "Authorize Slack in the browser.",
        credential_next_step: "After authorization completes, DM the Slack bot.",
      },
      isLoading: false,
      error: null,
    },
  });

  const body = JSON.stringify(rendered);
  assert.equal(oauthCalls.length, 0);
  assert.equal(openedPopups.length, 0);
  assert.match(body, /extensions\.authPopup/);
  assert.match(body, /extensions\.authorize/);
  assert.match(body, /Authorize Slack in the browser/);
  assert.doesNotMatch(body, /pairing\.placeholder/);
});

test("ConfigureModal does not show a generic activate action beside Slack OAuth", () => {
  const slackOauthSecret = {
    name: "slack_personal_oauth",
    provider: "slack_personal",
    prompt: "Slack credential",
    provided: true,
    setup: {
      kind: "oauth",
      account_label: "slack slack_personal",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  const { rendered } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    onboardingState: "setup_required",
    setupReady: true,
    setupResult: {
      secrets: [slackOauthSecret],
      fields: [],
      onboarding: {
        credential_instructions: "Authorize Slack in the browser.",
        credential_next_step: "After authorization completes, DM the Slack bot.",
      },
      isLoading: false,
      error: null,
    },
  });

  const body = JSON.stringify(rendered);
  assert.match(body, /extensions\.reconnect/);
  assert.doesNotMatch(body, /extensions\.activate/);
});

test("ConfigureModal broadcasts public wasm-tool Slack OAuth completion without duplicate activation", async () => {
  const slackOauthSecret = {
    name: "slack_personal_oauth",
    provider: "slack_personal",
    prompt: "Slack credential",
    provided: false,
    setup: {
      kind: "oauth",
      account_label: "slack slack_personal",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  let closed = false;
  const { calls, invalidations, notifications, oauthSetupArgs } = renderModal({
    kind: "wasm_tool",
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    onboardingState: "setup_required",
    setupResult: {
      secrets: [slackOauthSecret],
      fields: [],
      onboarding: {
        credential_instructions: "Authorize Slack in the browser.",
        credential_next_step: "After authorization completes, DM the Slack bot.",
      },
      isLoading: false,
      error: null,
    },
    onClose: () => {
      closed = true;
    },
  });

  assert.equal(oauthSetupArgs.length, 1);
  assert.equal(oauthSetupArgs[0][0]?.id, "slack");
  assert.equal(typeof oauthSetupArgs[0][1]?.onConfigured, "function");

  await oauthSetupArgs[0][1].onConfigured();

  assert.deepEqual(JSON.parse(JSON.stringify(calls)), []);
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["extensions"],
    ["extension-registry"],
    ["extension-setup", "slack"],
  ]);
  assert.equal(closed, true);
  // Connecting from the Extensions page must also resume any chat thread that
  // parked a request behind this channel's connection card (the same
  // channel-connected broadcast pairing redemption already sends).
  assert.deepEqual(JSON.parse(JSON.stringify(notifications)), [
    { channel: "slack", source: "extensions-oauth" },
  ]);
});

test("ConfigureModal surfaces a failed OAuth flow as a retryable error", () => {
  const { rendered } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    onboardingState: "setup_required",
    setupResult: {
      secrets: [
        {
          name: "slack_personal_oauth",
          provider: "slack_personal",
          provided: false,
          setup: { kind: "oauth", invocation_id: "invocation-alpha" },
        },
      ],
      fields: [],
      onboarding: null,
      isLoading: false,
      error: null,
    },
    oauthMutationState: { authError: "Authorization failed. Try connecting again." },
  });

  assert.ok(
    JSON.stringify(rendered).includes("Authorization failed. Try connecting again."),
    "the modal must render the watcher's flow-failure error",
  );
});

test("ConfigureModal closes as soon as Slack OAuth setup completes", async () => {
  const slackOauthSecret = {
    name: "slack_personal_oauth",
    provider: "slack_personal",
    prompt: "Slack credential",
    provided: false,
    setup: {
      kind: "oauth",
      account_label: "slack slack_personal",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  let closed = false;
  let releaseActivation;
  const activationGate = new Promise((resolve) => {
    releaseActivation = resolve;
  });
  const { oauthSetupArgs } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    onboardingState: "setup_required",
    setupResult: {
      secrets: [slackOauthSecret],
      fields: [],
      onboarding: {
        credential_instructions: "Authorize Slack in the browser.",
        credential_next_step: "After authorization completes, DM the Slack bot.",
      },
      isLoading: false,
      error: null,
    },
    activate: () => activationGate,
    onClose: () => {
      closed = true;
    },
  });

  const configuredPromise = oauthSetupArgs[0][1].onConfigured();
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(closed, true);

  releaseActivation();
  await configuredPromise;
});

test("ConfigureModal keeps Slack OAuth visibly loading while waiting for authorization", () => {
  const slackOauthSecret = {
    name: "slack_personal_oauth",
    provider: "slack_personal",
    prompt: "Slack credential",
    provided: false,
    setup: {
      kind: "oauth",
      account_label: "slack slack_personal",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  const { rendered } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    onboardingState: "setup_required",
    oauthMutationState: { isAuthorizing: true },
    setupResult: {
      secrets: [slackOauthSecret],
      fields: [],
      onboarding: {
        credential_instructions: "Authorize Slack in the browser.",
        credential_next_step: "After authorization completes, DM the Slack bot.",
      },
      isLoading: false,
      error: null,
    },
  });

  const body = JSON.stringify(rendered);
  // The shared Button owns the spinner via `loading`; the "opening" label +
  // the loading prop are what make the OAuth button visibly busy.
  assert.match(body, /,true,"extensions\.opening"/);
  assert.match(body, /extensions\.opening/);
  assert.doesNotMatch(body, /extensions\.authorize/);
});

test("ConfigureModal does not render the pairing panel for a non-channel extension", () => {
  const { rendered } = renderModal({ kind: "mcp_server" });
  const body = JSON.stringify(rendered);
  assert.doesNotMatch(body, /pairing\.placeholder/);
});

test("ConfigureModal does not route setup-required channels to the pairing panel", () => {
  const { rendered } = renderModal({
    kind: "channel",
    onboardingState: "setup_required",
  });

  assert.doesNotMatch(JSON.stringify(rendered), /pairing\.placeholder/);
});

test("ConfigureModal localizes channel pairing copy", () => {
  const { rendered } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "telegram" },
    channel: "telegram",
    displayName: "Telegram",
    onboardingState: "pairing_required",
    translate: (key) =>
      ({
        "extensions.configureName": "Configure {name}",
        "pairing.instructions": "Localized channel pairing instructions",
        "pairing.placeholder": "Localized channel pairing placeholder",
        "pairing.connect": "Localized connect",
        "common.saving": "Localized saving",
        "pairing.error": "Localized channel pairing error",
      })[key] || key,
  });
  const body = JSON.stringify(rendered);

  assert.match(body, /Localized channel pairing instructions/);
  assert.match(body, /Localized channel pairing placeholder/);
  assert.match(body, /Localized connect/);
  assert.doesNotMatch(body, /Open Slack and message/);
  assert.doesNotMatch(body, /Enter pairing code/);
});

test("ConfigureModal renders a localized close label through ModalShell", () => {
  const { context, rendered } = renderModal({
    kind: "channel",
    onboardingState: "pairing_required",
    translate: (key) =>
      ({
        "common.close": "Localized close",
        "extensions.configureName": "Configure {name}",
      })[key] || key,
  });

  const shell = renderFirstComponent(
    rendered,
    context.globalThis.__testExports.ModalShell,
  );

  assert.ok(shell, "the configure modal shell should render");
  assert.match(JSON.stringify(shell), /Localized close/);
});

test("ConfigureModal pairing redeems then activates, invalidates queries, and closes", async () => {
  let closed = false;
  const { calls, invalidations, mutationConfig } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "telegram" },
    channel: "telegram",
    displayName: "Telegram",
    onboardingState: "pairing_required",
    onClose: () => {
      closed = true;
    },
  });

  const result = await mutationConfig.mutationFn("A1B2C3");
  mutationConfig.onSuccess();

  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    ["redeem", "telegram", "A1B2C3"],
    ["activate", { id: "telegram" }],
  ]);
  assert.deepEqual(result, { success: true });
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["extensions"],
    ["connectable-channels"],
    ["pairing", "telegram"],
  ]);
  assert.equal(closed, true);
});

test("ConfigureModal pairing redeems by channel slug and activates package id", async () => {
  const { calls, invalidations, mutationConfig } = renderModal({
    kind: "channel",
    onboardingState: "pairing_required",
    packageRef: { kind: "extension", id: "telegram-host-package" },
    channel: "telegram",
    displayName: "Telegram",
  });

  await mutationConfig.mutationFn("A1B2C3");
  mutationConfig.onSuccess();

  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    ["redeem", "telegram", "A1B2C3"],
    ["activate", { id: "telegram-host-package" }],
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["extensions"],
    ["connectable-channels"],
    ["pairing", "telegram"],
  ]);
});

test("ConfigureModal pairing through the real API waits for blocked chats to resume", async () => {
  const originalWindow = globalThis.window;
  const originalFetch = globalThis.fetch;
  const originalSessionStorage = globalThis.sessionStorage;

  try {
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
      channel: "telegram",
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
      packageRef: { kind: "extension", id: "telegram" },
      channel: "telegram",
      displayName: "Telegram",
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
      channel: "telegram",
      code: "A1B2C3",
    });
    assert.equal(
      JSON.parse(fetches[1].options.body).content,
      "Telegram is connected. Continue the previous request.",
    );
    assert.equal(settled, false, "the modal mutation must wait for resume to finish");

    releaseResume();
    assert.deepEqual(await mutationPromise, {
      success: true,
      provider: "slack",
      provider_user_id: "install-alpha:U123",
      resumeError: false,
      resumedRunCount: 0,
    });
    assert.equal(settled, true);
  } finally {
    globalThis.window = originalWindow;
    globalThis.fetch = originalFetch;
    globalThis.sessionStorage = originalSessionStorage;
  }
});

test("ConfigureModal treats post-redeem activation failure as best-effort", async () => {
  const { calls, mutationConfig } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "telegram" },
    channel: "telegram",
    displayName: "Telegram",
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
    ["redeem", "telegram", "A1B2C3"],
    ["activate", { id: "telegram" }],
  ]);
});

test("ConfigureModal surfaces a blocked popup and does not start the OAuth flow", () => {
  const slackOauthSecret = {
    name: "slack_personal_oauth",
    provider: "slack_personal",
    prompt: "Slack credential",
    provided: false,
    setup: {
      kind: "oauth",
      account_label: "slack slack_personal",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  const { rendered, oauthCalls, openedPopups, stateSets } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    onboardingState: "pairing_required",
    blockPopup: true,
    setupResult: {
      secrets: [slackOauthSecret],
      fields: [],
      onboarding: {
        credential_instructions: "Authorize Slack in the browser.",
        credential_next_step: "After authorization completes, DM the Slack bot.",
      },
      isLoading: false,
      error: null,
    },
  });

  const authorize = findHandler(rendered, "handleOauth");
  assert.ok(authorize, "authorize click handler is rendered");
  authorize();

  assert.equal(openedPopups.length, 1, "the about:blank pre-open was attempted");
  assert.equal(openedPopups[0].popup, null);
  assert.equal(
    oauthCalls.length,
    0,
    "a blocked popup must not burn the server-side OAuth flow start"
  );
  assert.ok(
    stateSets.includes("Authorization popup was blocked."),
    "the blocked-popup error is surfaced to the modal"
  );
});

test("ConfigureModal starts the OAuth flow when the popup pre-open succeeds", () => {
  const slackOauthSecret = {
    name: "slack_personal_oauth",
    provider: "slack_personal",
    prompt: "Slack credential",
    provided: false,
    setup: {
      kind: "oauth",
      account_label: "slack slack_personal",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  const { rendered, oauthCalls, openedPopups } = renderModal({
    kind: "channel",
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    onboardingState: "pairing_required",
    setupResult: {
      secrets: [slackOauthSecret],
      fields: [],
      onboarding: {
        credential_instructions: "Authorize Slack in the browser.",
        credential_next_step: "After authorization completes, DM the Slack bot.",
      },
      isLoading: false,
      error: null,
    },
  });

  const authorize = findHandler(rendered, "handleOauth");
  assert.ok(authorize, "authorize click handler is rendered");
  authorize();

  assert.equal(oauthCalls.length, 1, "an unblocked pre-open starts the OAuth flow");
  assert.equal(openedPopups.length, 1);
  assert.equal(
    oauthCalls[0].popup.opener,
    null,
    "the pre-opened popup is passed with its opener severed"
  );
});
