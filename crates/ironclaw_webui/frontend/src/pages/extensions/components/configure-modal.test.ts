// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import {
  channelConnection,
  hasChannelSurface,
  isWebGeneratedCodeConnection,
} from "../lib/extensions-schema";

// Wire-shaped surface fixtures: a channel extension declares a channel
// surface; a plain tool extension declares only a tool surface.
const channelSurfaces = [
  {
    kind: "channel",
    inbound: true,
    outbound: true,
    connection: { strategy: "oauth" },
  },
];
const webCodeSurfaces = [
  {
    kind: "channel",
    inbound: true,
    outbound: true,
    connection: { strategy: "web_generated_code" },
  },
];
const toolSurfaces = [{ kind: "tool" }];

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
  surfaces = channelSurfaces,
  packageRef = { kind: "extension", id: "slack" },
  channel = undefined,
  displayName = "Slack",
  installationState = "setup_needed",
  onClose = () => {},
  onSaved,
  translate,
  setupResult,
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
  const context = {
    useQueryClient: () => ({
      invalidateQueries: ({ queryKey }) => invalidations.push(queryKey),
    }),
    PairingWebCodePanel() {},
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
    extensionIsActive: (extension) => extension?.installation_state === "active",
    // The real surface-taxonomy helpers: modal routing must key off declared
    // channel surfaces and connect strategies, exactly as production does.
    channelConnection,
    hasChannelSurface,
    isWebGeneratedCodeConnection,
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
      surfaces,
      channel,
      installation_state: installationState,
    },
    onClose,
    onSaved,
  });
  return {
    calls,
    context,
    PairingWebCodePanel: context.PairingWebCodePanel,
    invalidations,
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


function renderedContainsComponent(rendered, component) {
  if (!rendered || typeof rendered !== "object") {
    return rendered === component;
  }
  if (Array.isArray(rendered)) {
    return rendered.some((value) => renderedContainsComponent(value, component));
  }
  if (Array.isArray(rendered.values)) {
    return rendered.values.some((value) => renderedContainsComponent(value, component));
  }
  return false;
}

test("ConfigureModal hosts the web-code pairing panel instead of a paste box or no-config fallback", () => {
  const view = renderModal({
    surfaces: webCodeSurfaces,
    packageRef: { kind: "extension", id: "acme-messenger" },
    channel: "acme-messenger",
    displayName: "Acme Messenger",
    installationState: "setup_needed",
  });

  assert.equal(
    renderedContainsComponent(view.rendered, view.PairingWebCodePanel),
    true,
    "a manifest-declared WebGeneratedCode channel must render the minted-code pairing panel",
  );
  const body = JSON.stringify(view.rendered);
  assert.ok(!body.includes("pairing.placeholder"), "no proof-code paste box");
  assert.ok(
    !body.includes("extensions.noConfigRequired"),
    "web-code Configure must never claim no configuration is required",
  );
});

test("ConfigureModal keeps the web-code panel for an installed (non-pairing) lifecycle state", () => {
  const view = renderModal({
    surfaces: webCodeSurfaces,
    packageRef: { kind: "extension", id: "acme-messenger" },
    channel: "acme-messenger",
    displayName: "Acme Messenger",
    installationState: "setup_needed",
  });

  assert.equal(
    renderedContainsComponent(view.rendered, view.PairingWebCodePanel),
    true,
    "the empty-secrets fallback must not swallow the web-code panel",
  );
});

test("ConfigureModal renders Slack OAuth without opening the popup automatically", () => {
  const slackOauthSecret = {
    name: "slack_oauth",
    provider: "slack",
    prompt: "Slack credential",
    provided: false,
    setup: {
      kind: "oauth",
      account_label: "slack slack",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  const { rendered, oauthCalls, openedPopups } = renderModal({
    surfaces: channelSurfaces,
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    installationState: "setup_needed",
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

test("ConfigureModal never renders tenant administrator fields in caller setup", () => {
  const { rendered } = renderModal({
    surfaces: channelSurfaces,
    packageRef: { kind: "extension", id: "provider-neutral-channel" },
    channel: "provider-neutral-channel",
    displayName: "Provider Neutral Channel",
    installationState: "setup_needed",
    setupResult: {
      secrets: [
        {
          name: "personal_oauth",
          provider: "provider-neutral",
          prompt: "Connect your account",
          provided: false,
          setup: {
            kind: "oauth",
            account_label: "provider-neutral account",
            scopes: ["messages:read"],
            invocation_id: "invocation-personal-oauth",
          },
        },
      ],
      // A stale or mixed-version server must not make deployment-owned
      // manifest configuration editable on the caller's Configure surface.
      fields: [
        {
          name: "deployment_provider_id",
          prompt: "Tenant deployment provider id",
          optional: false,
        },
      ],
      onboarding: null,
      isLoading: false,
      error: null,
    },
  });

  const body = JSON.stringify(rendered);
  assert.match(body, /Connect your account/);
  assert.match(body, /extensions\.authorize/);
  assert.doesNotMatch(body, /Tenant deployment provider id/);
  assert.doesNotMatch(body, /deployment_provider_id/);
});

test("ConfigureModal does not show a generic activate action beside Slack OAuth", () => {
  const slackOauthSecret = {
    name: "slack_oauth",
    provider: "slack",
    prompt: "Slack credential",
    provided: true,
    setup: {
      kind: "oauth",
      account_label: "slack slack",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  const { rendered } = renderModal({
    surfaces: channelSurfaces,
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    installationState: "setup_needed",
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

test("ConfigureModal leaves post-OAuth lifecycle continuation to the server", async () => {
  const { calls, oauthSetupArgs } = renderModal({
    surfaces: toolSurfaces,
    packageRef: { kind: "extension", id: "provider-neutral-tool" },
    displayName: "Provider Neutral Tool",
    installationState: "setup_needed",
    setupResult: {
      secrets: [
        {
          name: "tool_oauth",
          provider: "vendor-a",
          provided: false,
          setup: { kind: "oauth", invocation_id: "invocation-server-owned" },
        },
      ],
      fields: [],
      onboarding: null,
      isLoading: false,
      error: null,
    },
  });

  await oauthSetupArgs[0][1].onConfigured();

  assert.deepEqual(
    JSON.parse(JSON.stringify(calls)),
    [],
    "OAuth completion must not rely on a best-effort browser activation call",
  );
});

test("ConfigureModal does not equate shared channel activation with personal connection", async () => {
  const { notifications, oauthSetupArgs } = renderModal({
    surfaces: channelSurfaces,
    packageRef: { kind: "extension", id: "channel-a" },
    channel: "channel-a",
    displayName: "Channel A",
    installationState: "active",
    setupResult: {
      secrets: [
        {
          name: "personal_oauth",
          provider: "vendor-a",
          provided: false,
          setup: { kind: "oauth", invocation_id: "invocation-personal" },
        },
      ],
      fields: [],
      onboarding: null,
      isLoading: false,
      error: null,
    },
  });

  await oauthSetupArgs[0][1].onConfigured();

  assert.deepEqual(
    JSON.parse(JSON.stringify(notifications)),
    [],
    "an active shared package is not proof that this caller's channel identity is connected",
  );
});

test("ConfigureModal skips the post-OAuth activation for an already-active extension", async () => {
  const { calls, oauthSetupArgs } = renderModal({
    surfaces: channelSurfaces,
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    installationState: "active",
    setupResult: {
      secrets: [
        {
          name: "slack_oauth",
          provider: "slack",
          provided: true,
          setup: { kind: "oauth", invocation_id: "invocation-alpha" },
        },
      ],
      fields: [],
      onboarding: null,
      isLoading: false,
      error: null,
    },
  });

  await oauthSetupArgs[0][1].onConfigured();

  assert.deepEqual(
    JSON.parse(JSON.stringify(calls)),
    [],
    "a reconnect on an active extension must not re-run activation",
  );
});

test("ConfigureModal surfaces a failed OAuth flow as a retryable error", () => {
  const { rendered } = renderModal({
    surfaces: channelSurfaces,
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    installationState: "setup_needed",
    setupResult: {
      secrets: [
        {
          name: "slack_oauth",
          provider: "slack",
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
    name: "slack_oauth",
    provider: "slack",
    prompt: "Slack credential",
    provided: false,
    setup: {
      kind: "oauth",
      account_label: "slack slack",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  let closed = false;
  const { oauthSetupArgs } = renderModal({
    surfaces: channelSurfaces,
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    installationState: "setup_needed",
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

  const configuredPromise = oauthSetupArgs[0][1].onConfigured();
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(closed, true);
  await configuredPromise;
});

test("ConfigureModal keeps Slack OAuth visibly loading while waiting for authorization", () => {
  const slackOauthSecret = {
    name: "slack_oauth",
    provider: "slack",
    prompt: "Slack credential",
    provided: false,
    setup: {
      kind: "oauth",
      account_label: "slack slack",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  const { rendered } = renderModal({
    surfaces: channelSurfaces,
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    installationState: "setup_needed",
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
  const { rendered } = renderModal({ surfaces: toolSurfaces });
  const body = JSON.stringify(rendered);
  assert.doesNotMatch(body, /pairing\.placeholder/);
});

test("ConfigureModal routes by manifest connection strategy, not a legacy onboarding state", () => {
  const view = renderModal({
    surfaces: webCodeSurfaces,
    installationState: "setup_needed",
  });

  assert.equal(
    renderedContainsComponent(view.rendered, view.PairingWebCodePanel),
    true,
  );
});

test("ConfigureModal renders a localized close label through ModalShell", () => {
  const { context, rendered } = renderModal({
    surfaces: channelSurfaces,
    installationState: "setup_needed",
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

test("ConfigureModal surfaces a blocked popup and does not start the OAuth flow", () => {
  const slackOauthSecret = {
    name: "slack_oauth",
    provider: "slack",
    prompt: "Slack credential",
    provided: false,
    setup: {
      kind: "oauth",
      account_label: "slack slack",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  const { rendered, oauthCalls, openedPopups, stateSets } = renderModal({
    surfaces: channelSurfaces,
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    installationState: "setup_needed",
    translate: (key) =>
      key === "authGate.popupBlocked"
        ? "La ventana emergente de autorización fue bloqueada."
        : key,
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
    stateSets.includes("La ventana emergente de autorización fue bloqueada."),
    "the blocked-popup error is surfaced to the modal in the selected language"
  );
});

test("ConfigureModal starts the OAuth flow when the popup pre-open succeeds", () => {
  const slackOauthSecret = {
    name: "slack_oauth",
    provider: "slack",
    prompt: "Slack credential",
    provided: false,
    setup: {
      kind: "oauth",
      account_label: "slack slack",
      scopes: ["users:read"],
      invocation_id: "invocation-alpha",
    },
  };
  const { rendered, oauthCalls, openedPopups } = renderModal({
    surfaces: channelSurfaces,
    packageRef: { kind: "extension", id: "slack" },
    channel: "slack",
    displayName: "Slack",
    installationState: "setup_needed",
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
