// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function authOauthCardSourceForTest() {
  const source = readFileSync(new URL("./auth-oauth-card.tsx", import.meta.url), "utf8");
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
    lines.push(line.replace("export function AuthOauthCard", "function AuthOauthCard"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { AuthOauthCard };`;
}

function renderCard({ gate, blockPopup = false } = {}) {
  const stateSets = [];
  const openCalls = [];
  const openAuthPopupCalls = [];
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    React: {
      useState: (initial) => [
        typeof initial === "function" ? initial() : initial,
        (next) => {
          stateSets.push(next);
        },
      ],
      useCallback: (fn) => fn,
      useMemo: (fn) => fn(),
      useEffect: () => {},
      useRef: (initial) => ({ current: initial ?? null }),
    },
    useT: () => (key, params) =>
      params ? `${key}:${JSON.stringify(params)}` : key,
    Button() {},
    Icon() {},
    Spinner() {},
    AuthGateShell() {},
    openAuthPopup: (url, popup) => {
      openAuthPopupCalls.push({ url, popup });
      // Mirror the production reuse path: an open popup is navigated in
      // place rather than replaced.
      if (popup && !popup.closed) {
        popup.location.href = url;
        return { ok: true, popup };
      }
      return { ok: true, popup: null, reason: null };
    },
    // Full-signature mock: production passes (url, target, features) and the
    // assertions below must cover every argument.
    window: {
      open: (url, target, features) => {
        if (blockPopup) {
          openCalls.push({ url, target, features, popup: null });
          return null;
        }
        const popup = {
          closed: false,
          opener: "test-opener",
          location: { href: url },
        };
        openCalls.push({ url, target, features, popup });
        return popup;
      },
    },
    URL,
  };

  vm.runInNewContext(authOauthCardSourceForTest(), context);
  const rendered = context.globalThis.__testExports.AuthOauthCard({
    gate,
    onCancel: () => {},
  });
  return { rendered, stateSets, openCalls, openAuthPopupCalls };
}

function defaultGate(overrides = {}) {
  return {
    provider: "slack_personal",
    authorizationUrl: "https://slack.com/oauth/v2/authorize?client_id=abc",
    gateRef: "gate-1",
    runId: "run-1",
    ...overrides,
  };
}

// Walks the rendered html-template tree and returns the first captured
// function whose source contains the marker (the openAuth click closure).
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

test("AuthOauthCard renders a display name, never the raw provider id", () => {
  const { rendered } = renderCard({ gate: defaultGate() });
  const body = JSON.stringify(rendered);
  assert.match(body, /Slack/);
  assert.doesNotMatch(body, /Slack_personal/i);
  assert.doesNotMatch(body, /slack_personal/);
});

test("AuthOauthCard prettifies unknown provider ids instead of leaking underscores", () => {
  const { rendered } = renderCard({
    gate: defaultGate({ provider: "acme_corp" }),
  });
  const body = JSON.stringify(rendered);
  assert.match(body, /Acme Corp/);
  assert.doesNotMatch(body, /acme_corp/);
});

test("AuthOauthCard opens authorization in a sized popup with the opener severed", () => {
  const { rendered, openCalls, openAuthPopupCalls, stateSets } = renderCard({
    gate: defaultGate(),
  });

  const openAuth = findHandler(rendered, "openAuth");
  assert.ok(openAuth, "openAuth click handler is rendered");
  openAuth({ preventDefault() {} });

  assert.equal(openCalls.length, 1);
  assert.equal(openCalls[0].url, "about:blank");
  assert.equal(openCalls[0].target, "_blank");
  assert.equal(
    openCalls[0].features,
    "width=600,height=600",
    "authorization must open as a sized popup, not a tab"
  );
  assert.equal(openCalls[0].popup.opener, null, "opener must be severed");
  assert.equal(openAuthPopupCalls.length, 1);
  assert.equal(
    openAuthPopupCalls[0].url,
    "https://slack.com/oauth/v2/authorize?client_id=abc"
  );
  assert.equal(
    openAuthPopupCalls[0].popup,
    openCalls[0].popup,
    "the pre-opened popup is navigated, not replaced"
  );
  assert.equal(
    openCalls[0].popup.location.href,
    "https://slack.com/oauth/v2/authorize?client_id=abc"
  );
  assert.ok(stateSets.includes(true), "opened state is set");
});

test("AuthOauthCard surfaces a blocked popup instead of silently doing nothing", () => {
  const { rendered, openAuthPopupCalls, stateSets } = renderCard({
    gate: defaultGate(),
    blockPopup: true,
  });

  const openAuth = findHandler(rendered, "openAuth");
  assert.ok(openAuth, "openAuth click handler is rendered");
  openAuth({ preventDefault() {} });

  assert.equal(openAuthPopupCalls.length, 0, "no navigation on a blocked popup");
  assert.ok(
    stateSets.includes("authGate.popupBlocked"),
    "blocked-popup error is surfaced through i18n"
  );
  assert.ok(!stateSets.includes(true), "opened state must not be set");
});

test("AuthOauthCard refuses non-HTTPS authorization urls before any window.open", () => {
  const { rendered, openCalls, openAuthPopupCalls, stateSets } = renderCard({
    gate: defaultGate({ authorizationUrl: "javascript:alert(1)" }),
  });

  const openAuth = findHandler(rendered, "openAuth");
  assert.ok(openAuth, "openAuth click handler is rendered");
  openAuth({ preventDefault() {} });

  assert.equal(openCalls.length, 0, "no window.open for a non-HTTPS url");
  assert.equal(openAuthPopupCalls.length, 0);
  assert.ok(stateSets.includes("authGate.serviceUnavailable"));
});

// Render with the four useState slots forced ([opened, error, closedNotice,
// watchNonce]) so we can exercise the waiting + closed-before-finish render
// states the no-op-setter default stub can't reach.
function renderCardWithStates(states, gate = defaultGate()) {
  let stateIndex = 0;
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    React: {
      useState: () => [states[stateIndex++], () => {}],
      useCallback: (fn) => fn,
      useMemo: (fn) => fn(),
      useEffect: () => {},
      useRef: (initial) => ({ current: initial ?? null }),
    },
    useT: () => (key, params) =>
      params ? `${key}:${JSON.stringify(params)}` : key,
    Button() {},
    Icon() {},
    Spinner() {},
    AuthGateShell() {},
    openAuthPopup: () => ({ ok: true, popup: null }),
    window: {
      open: () => ({ closed: false }),
      setInterval: () => 0,
      clearInterval() {},
      setTimeout: () => 0,
      clearTimeout() {},
    },
    URL,
  };
  vm.runInNewContext(authOauthCardSourceForTest(), context);
  return context.globalThis.__testExports.AuthOauthCard({ gate, onCancel: () => {} });
}

test("AuthOauthCard shows the button loading + waiting label while the popup is open", () => {
  // opened=true, closedNotice=false -> awaiting
  const body = JSON.stringify(renderCardWithStates([true, "", false, 1]));
  assert.match(body, /authGate\.authorizing/, "the button shows the waiting label");
  assert.doesNotMatch(body, /closed before you finished/, "no closed notice while awaiting");
});

test("AuthOauthCard surfaces closed-before-finish feedback after the popup closes", () => {
  // opened=true, closedNotice=true
  const body = JSON.stringify(renderCardWithStates([true, "", true, 1]));
  assert.match(body, /closed before you finished/, "closed-before-finish notice appears");
  assert.doesNotMatch(body, /authGate\.authorizing/, "no waiting label once closed");
});
