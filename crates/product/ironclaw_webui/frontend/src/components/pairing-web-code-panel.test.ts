// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

const BASE_MS = Date.parse("2026-07-16T12:00:00Z");
const LIVE_EXPIRES_AT = "2026-07-16T12:01:30Z"; // BASE + 90s
const STALE_EXPIRES_AT = "2026-07-16T11:59:00Z"; // already expired at BASE

function telegramPairingPanelSourceForTest() {
  const source = readFileSync(new URL("./pairing-web-code-panel.tsx", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { PairingWebCodePanel, formatPairingCountdown };`;
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function tForTest(key, params = {}) {
  if (key === "pairing.web.expiresIn") return `Expires in ${params.time}`;
  if (key === "pairing.web.qrAlt") return "Telegram pairing QR";
  return key;
}

function valuesAfter(rendered, fragment) {
  const matches = [];
  collectValuesAfter(rendered, fragment, matches);
  return matches;
}

function collectValuesAfter(value, fragment, matches) {
  if (Array.isArray(value)) {
    for (const item of value) collectValuesAfter(item, fragment, matches);
    return;
  }
  if (!value || !Array.isArray(value.strings) || !Array.isArray(value.values)) {
    return;
  }
  value.strings.forEach((part, index) => {
    if (part.includes(fragment)) {
      matches.push(value.values[index]);
    }
  });
  value.values.forEach((item) => collectValuesAfter(item, fragment, matches));
}

// React stub with full dependency-array comparison and cleanup support: the
// panel's intervals are registered/cleared through effect cleanups, so the
// harness must run them like React's commit phase does.
function createPanelHarness({ pairingResponses = [], startResponses = [], qrResults = [] } = {}) {
  const state = { hookIndex: 0, values: {}, refs: {}, effects: {}, pendingEffects: [] };
  const timers = { nextId: 1, active: new Map() };
  const notifyCalls = [];
  const invalidations = [];
  const clipboardWrites = [];
  const apiCalls = [];
  let nowMs = BASE_MS;

  const takeScripted = (queue, label) => {
    if (queue.length === 0) throw new Error(`no scripted response left for ${label}`);
    return queue.length > 1 ? queue.shift() : queue[0];
  };

  const context = {
    Button: "button",
    globalThis: {},
    Date: { now: () => nowMs, parse: (value) => Date.parse(value) },
    navigator: {
      clipboard: {
        writeText: async (text) => {
          clipboardWrites.push(text);
        },
      },
    },
    setInterval: (fn, ms) => {
      const id = timers.nextId++;
      timers.active.set(id, { fn, ms });
      return id;
    },
    clearInterval: (id) => timers.active.delete(id),
    setTimeout: (fn, ms) => {
      const id = timers.nextId++;
      timers.active.set(id, { fn, ms, timeout: true });
      return id;
    },
    clearTimeout: (id) => timers.active.delete(id),
    QRCode: {
      toDataURL: async (text) => {
        apiCalls.push(["qr", text]);
        return takeScripted(qrResults, "QRCode.toDataURL");
      },
    },
    React: {
      useState: (initial) => {
        const index = state.hookIndex++;
        if (!(index in state.values)) {
          state.values[index] = typeof initial === "function" ? initial() : initial;
        }
        return [
          state.values[index],
          (next) => {
            state.values[index] =
              typeof next === "function" ? next(state.values[index]) : next;
          },
        ];
      },
      useRef: (initial) => {
        const index = state.hookIndex++;
        if (!(index in state.refs)) {
          state.refs[index] = { current: initial };
        }
        return state.refs[index];
      },
      useEffect: (effect, deps) => {
        const index = state.hookIndex++;
        const previous = state.effects[index];
        const changed =
          !previous ||
          !deps ||
          !previous.deps ||
          deps.length !== previous.deps.length ||
          deps.some((dep, position) => !Object.is(dep, previous.deps[position]));
        if (changed) {
          state.pendingEffects.push({ index, effect, deps: deps ? Array.from(deps) : deps });
        }
      },
    },
    useQueryClient: () => ({
      invalidateQueries: (query) => invalidations.push(query.queryKey),
    }),
    useT: () => tForTest,
    notifyChannelConnected: (payload) => {
      notifyCalls.push(payload);
      return Promise.resolve([]);
    },
    getExtensionPairingStatus: async () => {
      apiCalls.push(["get"]);
      return takeScripted(pairingResponses, "getExtensionPairingStatus");
    },
    mintExtensionPairingCode: async () => {
      apiCalls.push(["start"]);
      const value = takeScripted(startResponses, "mintExtensionPairingCode");
      if (value && value.__reject) throw value.__reject;
      return value;
    },
    unpairExtension: async () => {
      apiCalls.push(["disconnect"]);
    },
    extensionPairingError: (error, fallback) => error?.payload?.error || error?.message || fallback,
  };
  vm.runInNewContext(telegramPairingPanelSourceForTest(), context);

  const render = (props = {}) => {
    state.hookIndex = 0;
    const rendered = context.globalThis.__testExports.PairingWebCodePanel({
      extensionId: "telegram",
      ...props,
    });
    const queue = state.pendingEffects.splice(0);
    for (const { index, effect, deps } of queue) {
      state.effects[index]?.cleanup?.();
      const cleanup = effect();
      state.effects[index] = { deps, cleanup: typeof cleanup === "function" ? cleanup : null };
    }
    return rendered;
  };

  return {
    render,
    fireTimers: (ms) =>
      Promise.all(
        Array.from(timers.active.values())
          .filter((timer) => timer.ms === ms)
          .map((timer) => timer.fn()),
      ),
    setNow: (value) => {
      nowMs = value;
    },
    timers,
    notifyCalls,
    invalidations,
    clipboardWrites,
    apiCalls,
    context,
  };
}

test("formatPairingCountdown formats the remaining lifetime", () => {
  const harness = createPanelHarness();
  const { formatPairingCountdown } = harness.context.globalThis.__testExports;

  assert.equal(formatPairingCountdown(90_000), "1:30");
  assert.equal(formatPairingCountdown(5_000), "0:05");
  assert.equal(formatPairingCountdown(-1), "0:00");
});

test("PairingWebCodePanel renders only manifest-backed code, link, QR, and countdown data", async () => {
  const harness = createPanelHarness({
    pairingResponses: [
      {
        connected: false,
        pending: { code: "OLD-1", deep_link: "https://t.me/ironclaw_bot?start=OLD-1", expires_at: STALE_EXPIRES_AT },
      },
    ],
    startResponses: [
      {
        code: "TG-PAIR-42",
        deep_link: "https://t.me/ironclaw_bot?start=TG-PAIR-42",
        expires_at: LIVE_EXPIRES_AT,
      },
    ],
    qrResults: ["data:image/png;base64,QR1"],
  });

  harness.render();
  await tick();
  harness.render();
  await tick();
  const rendered = harness.render();

  // Stale server code is not reused: a fresh one is minted.
  assert.deepEqual(
    harness.apiCalls.filter((call) => call[0] !== "qr"),
    [["get"], ["start"]],
  );
  const body = JSON.stringify(rendered);
  assert.ok(body.includes("TG-PAIR-42"), "renders the pairing code");
  assert.ok(
    !body.includes("@ironclaw_bot"),
    "the generic panel must not infer provider-specific identity from a URL",
  );
  assert.ok(body.includes("Expires in 1:30"), "renders the countdown");
  assert.deepEqual(valuesAfter(rendered, "href="), ["https://t.me/ironclaw_bot?start=TG-PAIR-42"]);
  assert.ok(valuesAfter(rendered, "target=").includes("_blank"));
  assert.deepEqual(valuesAfter(rendered, "src="), ["data:image/png;base64,QR1"]);
  assert.ok(body.includes("Telegram pairing QR"), "QR image carries its alt text");
});

test("PairingWebCodePanel copies only the manifest-backed pairing code", async () => {
  const harness = createPanelHarness({
    pairingResponses: [
      {
        connected: false,
        pending: {
          code: "TG-PAIR-42",
          deep_link: "https://t.me/ironclaw_bot?start=TG-PAIR-42",
          expires_at: LIVE_EXPIRES_AT,
        },
      },
    ],
    qrResults: ["data:image/png;base64,QR1"],
  });

  harness.render();
  await tick();
  harness.render();
  await tick();
  const rendered = harness.render();

  const clicks = valuesAfter(rendered, "onClick=");
  await clicks[0]();

  assert.deepEqual(harness.clipboardWrites, ["TG-PAIR-42"]);
});

test("PairingWebCodePanel flips to the renewal state at expiry and renewal re-renders a fresh code + QR", async () => {
  const harness = createPanelHarness({
    pairingResponses: [
      {
        connected: false,
        pending: {
          code: "TG-PAIR-42",
          deep_link: "https://t.me/ironclaw_bot?start=TG-PAIR-42",
          expires_at: LIVE_EXPIRES_AT,
        },
      },
    ],
    startResponses: [
      {
        code: "TG-PAIR-99",
        deep_link: "https://t.me/ironclaw_bot?start=TG-PAIR-99",
        expires_at: "2026-07-16T12:05:00Z",
      },
    ],
    qrResults: ["data:image/png;base64,QR1", "data:image/png;base64,QR2"],
  });

  harness.render();
  await tick();
  harness.render();
  await tick();
  harness.render();

  // The countdown tick observes the passed expiry and flips the panel.
  harness.setNow(BASE_MS + 91_000);
  await harness.fireTimers(1000);
  const expiredView = harness.render();
  const expiredBody = JSON.stringify(expiredView);
  assert.ok(expiredBody.includes("pairing.web.expired"));
  assert.ok(expiredBody.includes("pairing.web.getNewCode"));
  assert.ok(!expiredBody.includes("TG-PAIR-42"), "expired state hides the dead code");
  assert.deepEqual(valuesAfter(expiredView, "src="), [], "expired state hides the QR");

  // Renewal mints a fresh code and re-renders a fresh QR for it.
  await valuesAfter(expiredView, "onClick=")[0]();
  harness.render();
  await tick();
  const renewedView = harness.render();
  const renewedBody = JSON.stringify(renewedView);
  assert.ok(renewedBody.includes("TG-PAIR-99"));
  assert.ok(!renewedBody.includes("TG-PAIR-42"));
  assert.deepEqual(valuesAfter(renewedView, "src="), ["data:image/png;base64,QR2"]);
  assert.deepEqual(harness.apiCalls.filter((call) => call[0] === "qr"), [
    ["qr", "https://t.me/ironclaw_bot?start=TG-PAIR-42"],
    ["qr", "https://t.me/ironclaw_bot?start=TG-PAIR-99"],
  ]);
});

test("PairingWebCodePanel poll flip to connected broadcasts the connection and invalidates channel caches", async () => {
  const harness = createPanelHarness({
    pairingResponses: [
      {
        connected: false,
        pending: {
          code: "TG-PAIR-42",
          deep_link: "https://t.me/ironclaw_bot?start=TG-PAIR-42",
          expires_at: LIVE_EXPIRES_AT,
        },
      },
      { connected: true, pending: null },
    ],
    qrResults: ["data:image/png;base64,QR1"],
  });

  harness.render();
  await tick();
  harness.render();
  await tick();
  harness.render();

  // An unexpired server-side pending code is reused, never re-minted.
  assert.ok(!harness.apiCalls.some((call) => call[0] === "start"));
  assert.ok(harness.timers.active.size > 0, "poll + countdown timers run while unconnected");

  await harness.fireTimers(2000);
  const connectedView = harness.render();

  const body = JSON.stringify(connectedView);
  assert.ok(body.includes("pairing.web.paired"));
  assert.ok(body.includes("✅"));
  assert.deepEqual(JSON.parse(JSON.stringify(harness.notifyCalls)), [
    { channel: "telegram", source: "pairing-web-code-panel" },
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(harness.invalidations)), [
    ["extensions"],
    ["connectable-channels"],
  ]);
  assert.equal(harness.timers.active.size, 0, "connected state stops polling and countdown");
});

test("PairingWebCodePanel disconnect DELETEs the pairing then mints a fresh code, without a connect broadcast", async () => {
  const harness = createPanelHarness({
    pairingResponses: [{ connected: true, pending: null }],
    startResponses: [
      {
        code: "TG-PAIR-77",
        deep_link: "https://t.me/ironclaw_bot?start=TG-PAIR-77",
        expires_at: LIVE_EXPIRES_AT,
      },
    ],
    qrResults: ["data:image/png;base64,QR3"],
  });

  harness.render();
  await tick();
  const connectedView = harness.render();
  assert.ok(JSON.stringify(connectedView).includes("pairing.web.paired"));
  // Mounting over an already-paired account is not a connection event.
  assert.deepEqual(harness.notifyCalls, []);

  await valuesAfter(connectedView, "onClick=")[0]();
  harness.render();
  await tick();
  const repairView = harness.render();

  assert.deepEqual(
    harness.apiCalls.filter((call) => call[0] === "disconnect" || call[0] === "start"),
    [["disconnect"], ["start"]],
  );
  assert.deepEqual(JSON.parse(JSON.stringify(harness.invalidations)), [
    ["extensions"],
    ["connectable-channels"],
  ]);
  assert.deepEqual(harness.notifyCalls, [], "disconnect never broadcasts a connect event");
  assert.ok(JSON.stringify(repairView).includes("TG-PAIR-77"), "a fresh code renders for re-pairing");
});

test("PairingWebCodePanel ignores an old pairing poll that resolves after disconnect", async () => {
  let resolveStalePoll;
  const stalePoll = new Promise((resolve) => {
    resolveStalePoll = resolve;
  });
  const harness = createPanelHarness({
    pairingResponses: [
      {
        connected: false,
        pending: {
          code: "TG-PAIR-42",
          deep_link: "https://t.me/ironclaw_bot?start=TG-PAIR-42",
          expires_at: LIVE_EXPIRES_AT,
        },
      },
      { connected: true, pending: null },
      stalePoll,
    ],
    startResponses: [
      {
        code: "TG-PAIR-88",
        deep_link: "https://t.me/ironclaw_bot?start=TG-PAIR-88",
        expires_at: LIVE_EXPIRES_AT,
      },
    ],
    qrResults: ["data:image/png;base64,QR1", "data:image/png;base64,QR2"],
  });

  harness.render();
  await tick();
  harness.render();
  await tick();
  harness.render();

  // Model overlapping interval callbacks: the first observes connection while
  // the second remains in flight with a snapshot from the same pairing epoch.
  const connectionPoll = harness.fireTimers(2000);
  const oldInFlightPoll = harness.fireTimers(2000);
  await connectionPoll;
  const connectedView = harness.render();
  assert.ok(JSON.stringify(connectedView).includes("pairing.web.paired"));

  await valuesAfter(connectedView, "onClick=")[0]();
  harness.render();
  await tick();
  let disconnectedView = harness.render();
  assert.ok(JSON.stringify(disconnectedView).includes("TG-PAIR-88"));

  resolveStalePoll({ connected: true, pending: null });
  await oldInFlightPoll;
  disconnectedView = harness.render();

  assert.ok(
    !JSON.stringify(disconnectedView).includes("pairing.web.paired"),
    "the pre-disconnect poll cannot restore the old connected state",
  );
  assert.ok(JSON.stringify(disconnectedView).includes("TG-PAIR-88"));
  assert.equal(
    harness.notifyCalls.length,
    1,
    "the stale poll does not broadcast a second connection event",
  );
});

test("PairingWebCodePanel reports a failed mint after a successful disconnect as a load error, not a failed disconnect", async () => {
  const harness = createPanelHarness({
    pairingResponses: [{ connected: true, pending: null }],
    startResponses: [{ __reject: { payload: { error: "code mint exploded" } } }],
  });

  harness.render();
  await tick();
  const connectedView = harness.render();
  assert.ok(JSON.stringify(connectedView).includes("pairing.web.paired"));

  await valuesAfter(connectedView, "onClick=")[0]();
  const view = harness.render();
  const body = JSON.stringify(view);

  // The DELETE succeeded: the account is disconnected, so the disconnect
  // failure copy must not appear — the surfaced error is the mint failure.
  assert.ok(!body.includes("pairing.web.paired"), "the account is disconnected");
  assert.ok(body.includes("code mint exploded"), "the mint failure surfaces");
  assert.ok(
    !body.includes("pairing.web.disconnectFailed"),
    "a mint failure is never reported as a failed disconnect",
  );
  assert.deepEqual(
    harness.apiCalls.filter((call) => call[0] === "disconnect" || call[0] === "start"),
    [["disconnect"], ["start"]],
  );
});
