// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";

import { sourceForVmTest } from "../test-support/vm-module-harness";

const BASE_MS = Date.parse("2026-07-16T12:00:00Z");
const LIVE_EXPIRES_AT_MS = BASE_MS + 90_000;

const LABELS = {
  qrAlt: "Example link QR",
  copy: "Copy code",
  copied: "Copied",
  open: "Open in Example",
  expiresIn: (time) => `Expires in ${time}`,
  expired: "This code expired.",
  renew: "Get a new code",
};

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function createHarness({ qrResults = [] } = {}) {
  const state = { hookIndex: 0, values: {}, refs: {}, effects: {}, pendingEffects: [] };
  const timers = { nextId: 1, active: new Map() };
  const clipboardWrites = [];
  const qrCalls = [];
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
        qrCalls.push(text);
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
        if (!(index in state.refs)) state.refs[index] = { current: initial };
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
  };
  vm.runInNewContext(
    sourceForVmTest(
      "./link-payload-panel.tsx",
      ["LinkPayloadPanel", "formatLinkCountdown"],
      import.meta.url,
    ),
    context,
  );

  const render = (props = {}) => {
    state.hookIndex = 0;
    const rendered = context.globalThis.__testExports.LinkPayloadPanel({
      labels: LABELS,
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
    exports: context.globalThis.__testExports,
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
    clipboardWrites,
    qrCalls,
  };
}

function valuesAfter(rendered, fragment) {
  const matches = [];
  collect(rendered, fragment, matches);
  return matches;
}

function collect(value, fragment, matches) {
  if (Array.isArray(value)) {
    for (const item of value) collect(item, fragment, matches);
    return;
  }
  if (!value || !Array.isArray(value.strings) || !Array.isArray(value.values)) return;
  value.strings.forEach((part, index) => {
    if (part.includes(fragment)) matches.push(value.values[index]);
  });
  value.values.forEach((item) => collect(item, fragment, matches));
}

test("formatLinkCountdown formats the remaining lifetime", () => {
  const { formatLinkCountdown } = createHarness().exports;

  assert.equal(formatLinkCountdown(90_000), "1:30");
  assert.equal(formatLinkCountdown(5_000), "0:05");
  assert.equal(formatLinkCountdown(-1), "0:00");
});

test("LinkPayloadPanel renders the QR, the code, the open affordance, and the countdown", async () => {
  const harness = createHarness({ qrResults: ["data:image/png;base64,QR1"] });

  harness.render({
    payload: "scheme://login?token=AAAA",
    code: "AB-CD-12",
    expiresAtMs: LIVE_EXPIRES_AT_MS,
  });
  await tick();
  const rendered = harness.render({
    payload: "scheme://login?token=AAAA",
    code: "AB-CD-12",
    expiresAtMs: LIVE_EXPIRES_AT_MS,
  });

  const body = JSON.stringify(rendered);
  assert.deepEqual(harness.qrCalls, ["scheme://login?token=AAAA"]);
  assert.deepEqual(valuesAfter(rendered, "src="), ["data:image/png;base64,QR1"]);
  assert.ok(body.includes("Example link QR"), "the QR image carries its alt text");
  assert.ok(body.includes("AB-CD-12"), "the code is rendered");
  assert.deepEqual(valuesAfter(rendered, "href="), ["scheme://login?token=AAAA"]);
  assert.ok(body.includes("Expires in 1:30"), "the countdown is rendered");
});

test("LinkPayloadPanel copies only the code, never the payload", async () => {
  const harness = createHarness({ qrResults: ["data:image/png;base64,QR1"] });
  const props = {
    payload: "scheme://login?token=AAAA",
    code: "AB-CD-12",
    expiresAtMs: LIVE_EXPIRES_AT_MS,
  };

  harness.render(props);
  await tick();
  const rendered = harness.render(props);

  await valuesAfter(rendered, "onClick=")[0]();
  assert.deepEqual(harness.clipboardWrites, ["AB-CD-12"]);
});

test("LinkPayloadPanel flips to the renewal view at expiry, stops ticking, and notifies once", async () => {
  const harness = createHarness({ qrResults: ["data:image/png;base64,QR1"] });
  const expiries = [];
  const renewals = [];
  const props = {
    payload: "scheme://login?token=AAAA",
    code: "AB-CD-12",
    expiresAtMs: LIVE_EXPIRES_AT_MS,
    onExpire: () => expiries.push("expired"),
    onRenew: () => renewals.push("renew"),
  };

  harness.render(props);
  await tick();
  harness.render(props);
  assert.ok(harness.timers.active.size > 0, "a live payload ticks the countdown");

  harness.setNow(BASE_MS + 91_000);
  await harness.fireTimers(1000);
  const expiredView = harness.render(props);
  const expiredBody = JSON.stringify(expiredView);

  assert.ok(expiredBody.includes("This code expired."));
  assert.ok(expiredBody.includes("Get a new code"));
  assert.ok(!expiredBody.includes("AB-CD-12"), "the expired view hides the dead code");
  assert.deepEqual(valuesAfter(expiredView, "src="), [], "the expired view hides the QR");
  assert.equal(harness.timers.active.size, 0, "an expired payload holds no timer");
  assert.deepEqual(expiries, ["expired"], "expiry notifies exactly once");

  // Re-rendering the same expired deadline must not re-notify.
  harness.render(props);
  assert.deepEqual(expiries, ["expired"]);

  await valuesAfter(expiredView, "onClick=")[0]();
  assert.deepEqual(renewals, ["renew"]);
});

test("LinkPayloadPanel renders a payload that is not meant to be scanned without a QR", async () => {
  // A payload the owner knows is a link to OPEN has nothing to scan: no QR
  // image, and the encoder is never asked for one.
  const harness = createHarness({ qrResults: ["data:image/png;base64,QR1"] });
  const props = {
    payload: "https://vendor.example/link/AAAA",
    showQr: false,
    expiresAtMs: LIVE_EXPIRES_AT_MS,
  };

  harness.render(props);
  await tick();
  const rendered = harness.render(props);

  assert.deepEqual(harness.qrCalls, [], "a payload that is not scanned is never encoded");
  assert.deepEqual(valuesAfter(rendered, "src="), []);
  assert.deepEqual(
    valuesAfter(rendered, "href="),
    ["https://vendor.example/link/AAAA"],
    "the affordance that opens it survives",
  );
});

test("LinkPayloadPanel renders a code-only payload without a QR or an open affordance", async () => {
  const harness = createHarness();
  const props = { code: "AB-CD-12", expiresAtMs: LIVE_EXPIRES_AT_MS };

  harness.render(props);
  await tick();
  const rendered = harness.render(props);

  assert.deepEqual(harness.qrCalls, [], "no payload means no QR render");
  assert.deepEqual(valuesAfter(rendered, "src="), []);
  assert.deepEqual(valuesAfter(rendered, "href="), []);
  assert.ok(JSON.stringify(rendered).includes("AB-CD-12"));
});
