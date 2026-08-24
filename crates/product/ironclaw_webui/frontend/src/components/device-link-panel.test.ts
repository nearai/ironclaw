// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";
import vm from "node:vm";

import { componentProps, findComponent } from "../lib/vm-component-harness";
import { sourceForVmTest } from "../test-support/vm-module-harness";
import {
  DEVICE_LINK_DISPLAY_KINDS,
  DEVICE_LINK_ERROR_CODES,
  DEVICE_LINK_INPUT_KINDS,
  DEVICE_LINK_MODES,
  DEVICE_LINK_STEPS,
  deviceLinkAlternateMode,
  deviceLinkFrameFromWire,
  deviceLinkModeLabel,
  deviceLinkPollDelayMs,
} from "../lib/device-link-frame";

const EXPIRES_AT = "2026-07-16T12:01:30Z";

// Wire frames, shaped exactly like `DeviceLinkPromptView`
// (crates/contracts/ironclaw_extension_contracts/src/auth_prompt.rs).
function wireFrame(overrides = {}) {
  return {
    provider: "example",
    display_name: "Example",
    step: DEVICE_LINK_STEPS.display,
    instructions: "Scan this from your device settings.",
    expires_at: EXPIRES_AT,
    revision: 1,
    poll_interval_ms: 3000,
    ...overrides,
  };
}

// A frame from an extension that declares a SECOND linking path. Always
// explicit: the wire default is "no alternate", and the card must not invent
// one — a vendor with a single path answers `UnsupportedMode` to a switch.
function alternateOffered(overrides = {}) {
  return wireFrame({ alternate_available: true, ...overrides });
}

function response(frame, { flowId = "flow-1" } = {}) {
  return { flow_id: flowId, status: "pending", device_link: frame };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function tForTest(key, params = {}) {
  const translations = {
    "deviceLink.personalDisclosure":
      "Link your personal {name} account as a third-party device. When you ask IronClaw, it can read your {name} chats and send messages that recipients see as coming from you. Revoke access at any time in {name} Settings → Devices.",
  };
  return Object.entries(params).reduce(
    (text, [name, value]) => text.replaceAll(`{${name}}`, String(value)),
    translations[key] || key,
  );
}

function createHarness({ startResponses = [], pollResponses = [], submitResponses = [] } = {}) {
  const state = { hookIndex: 0, values: {}, refs: {}, effects: {}, pendingEffects: [] };
  const timers = { nextId: 1, active: new Map() };
  const calls = [];
  // Whole request objects, kept alongside the compact `calls` tuples: scope
  // fields never appear in those tuples, and an argument the production caller
  // passes has to be captured to be asserted on.
  const requests = [];
  const completions = [];

  const takeScripted = (queue, label) => {
    if (queue.length === 0) throw new Error(`no scripted response left for ${label}`);
    const value = queue.length > 1 ? queue.shift() : queue[0];
    if (value && value.__reject) throw value.__reject;
    return value;
  };

  const context = {
    Button: "button",
    LinkPayloadPanel() {},
    globalThis: {},
    setInterval: (fn, ms) => {
      const id = timers.nextId++;
      timers.active.set(id, { fn, ms });
      return id;
    },
    clearInterval: (id) => timers.active.delete(id),
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
    useT: () => tForTest,
    // The real normalizer and the real vocabulary: a test that stubbed these
    // would not prove the card and the wire agree.
    DEVICE_LINK_DISPLAY_KINDS,
    DEVICE_LINK_ERROR_CODES,
    DEVICE_LINK_INPUT_KINDS,
    DEVICE_LINK_MODES,
    DEVICE_LINK_STEPS,
    deviceLinkAlternateMode,
    deviceLinkFrameFromWire,
    deviceLinkModeLabel,
    deviceLinkPollDelayMs,
    startDeviceLink: async (request) => {
      calls.push(["start", request.mode]);
      requests.push(["start", request]);
      return takeScripted(startResponses, "startDeviceLink");
    },
    pollDeviceLink: async (request) => {
      calls.push(["poll", request.flowId]);
      requests.push(["poll", request]);
      return takeScripted(pollResponses, "pollDeviceLink");
    },
    submitDeviceLinkInput: async (request) => {
      calls.push(["submit", request.kind, request.revision, request.value]);
      requests.push(["submit", request]);
      return takeScripted(submitResponses, "submitDeviceLinkInput");
    },
    cancelDeviceLink: async (request) => {
      calls.push(["cancel", request.flowId]);
      requests.push(["cancel", request]);
      return {};
    },
    deviceLinkError: (error, fallback) => error?.payload?.error || error?.message || fallback,
  };
  vm.runInNewContext(
    sourceForVmTest("./device-link-panel.tsx", ["DeviceLinkPanel"], import.meta.url),
    context,
  );

  const render = (props = {}) => {
    state.hookIndex = 0;
    const rendered = context.globalThis.__testExports.DeviceLinkPanel({
      provider: "example",
      extensionName: "example",
      displayName: "Example",
      onCompleted: (frame) => completions.push(frame.step),
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

  // Mount + settle the start call, then return the painted tree.
  const mount = async (props = {}) => {
    render(props);
    await tick();
    render(props);
    await tick();
    return render(props);
  };

  return {
    render,
    mount,
    calls,
    requests,
    completions,
    timers,
    context,
    payloadPanelProps: (rendered) => {
      const node = findComponent(rendered, context.LinkPayloadPanel);
      return node ? componentProps(node, context.LinkPayloadPanel) : null;
    },
    fireTimers: (ms) =>
      Promise.all(
        Array.from(timers.active.values())
          .filter((timer) => timer.ms === ms)
          .map((timer) => timer.fn()),
      ),
  };
}

function stringify(rendered) {
  return JSON.stringify(rendered);
}

function attribute(rendered, name) {
  const matches = [];
  const walk = (value) => {
    if (Array.isArray(value)) {
      value.forEach(walk);
      return;
    }
    if (!value || !Array.isArray(value.strings) || !Array.isArray(value.values)) return;
    value.strings.forEach((part, index) => {
      if (part.endsWith(`${name}=`)) matches.push(value.values[index]);
    });
    value.values.forEach(walk);
  };
  walk(rendered);
  return matches;
}

test("DeviceLinkPanel renders the QR display step through the shared payload panel", async () => {
  const harness = createHarness({
    startResponses: [response(wireFrame({ qr_payload: "scheme://login?token=AAAA" }))],
    pollResponses: [response(wireFrame({ qr_payload: "scheme://login?token=AAAA" }))],
  });

  const rendered = await harness.mount();

  assert.deepEqual(harness.calls[0], ["start", DEVICE_LINK_MODES.default]);
  assert.deepEqual(attribute(rendered, "data-device-link-step"), [DEVICE_LINK_STEPS.display]);
  const payload = harness.payloadPanelProps(rendered);
  assert.ok(payload, "the display step renders the shared payload panel");
  assert.equal(payload.payload, "scheme://login?token=AAAA");
  assert.equal(payload.idPrefix, "device-link");
  assert.equal(payload.expiresAtMs, Date.parse(EXPIRES_AT));
  assert.ok(stringify(rendered).includes("Scan this from your device settings."));
});

test("DeviceLinkPanel explains personal-account authority in every entry point", async () => {
  const harness = createHarness({
    startResponses: [response(wireFrame({ qr_payload: "scheme://login?token=AAAA" }))],
  });
  const props = { displayName: "Telegram" };
  const expectedDisclosure =
    "Link your personal Telegram account as a third-party device. When you ask IronClaw, it can read your Telegram chats and send messages that recipients see as coming from you. Revoke access at any time in Telegram Settings → Devices.";

  const loading = harness.render(props);
  assert.ok(
    stringify(loading).includes(expectedDisclosure),
    "the loading state must disclose the personal-account authority before linking starts",
  );

  const rendered = await harness.mount(props);
  assert.ok(
    stringify(rendered).includes(expectedDisclosure),
    "the settled shared panel carries the disclosure into Extensions, chat, and onboarding",
  );
});

test("DeviceLinkPanel renders the awaiting-vendor step and paces polling from the vendor back-off", async () => {
  const harness = createHarness({
    startResponses: [
      response(
        wireFrame({
          step: DEVICE_LINK_STEPS.awaitingVendor,
          instructions: "Waiting for confirmation.",
          retry_after_ms: 30_000,
        }),
      ),
    ],
    pollResponses: [response(wireFrame({ step: DEVICE_LINK_STEPS.awaitingVendor, revision: 2 }))],
  });

  const rendered = await harness.mount();

  assert.deepEqual(attribute(rendered, "data-device-link-step"), [
    DEVICE_LINK_STEPS.awaitingVendor,
  ]);
  assert.ok(stringify(rendered).includes("deviceLink.awaiting"));
  assert.deepEqual(
    Array.from(harness.timers.active.values()).map((timer) => timer.ms),
    [30_000],
    "a vendor back-off overrides the default poll pace",
  );
});

test("DeviceLinkPanel renders each input step with the affordance its kind requires", async () => {
  for (const [kind, expected] of [
    [DEVICE_LINK_INPUT_KINDS.identifier, { type: "tel", autoComplete: "tel" }],
    [DEVICE_LINK_INPUT_KINDS.code, { type: "text", autoComplete: "one-time-code" }],
    [DEVICE_LINK_INPUT_KINDS.password, { type: "password", autoComplete: "current-password" }],
  ]) {
    const frame = alternateOffered({
      step: DEVICE_LINK_STEPS.inputRequired,
      input_kind: kind,
      secret_label: `label for ${kind}`,
    });
    const harness = createHarness({
      startResponses: [response(frame)],
      pollResponses: [response(frame)],
    });

    const rendered = await harness.mount();

    assert.deepEqual(attribute(rendered, "data-device-link-input-kind"), [kind]);
    // field type, then the submit button, then the mode-switch button.
    assert.deepEqual(attribute(rendered, "type"), [expected.type, "submit", "button"]);
    assert.deepEqual(attribute(rendered, "autoComplete"), [expected.autoComplete]);
    assert.ok(
      stringify(rendered).includes(`label for ${kind}`),
      "the host-authored label is what the field is titled",
    );
  }
});

test("DeviceLinkPanel submits an input with the revision it was rendered from and clears the value", async () => {
  const inputFrame = wireFrame({
    step: DEVICE_LINK_STEPS.inputRequired,
    input_kind: DEVICE_LINK_INPUT_KINDS.password,
    secret_label: "Account password",
    revision: 4,
  });
  const harness = createHarness({
    startResponses: [response(inputFrame)],
    pollResponses: [response(inputFrame)],
    submitResponses: [
      response(
        wireFrame({
          step: DEVICE_LINK_STEPS.completed,
          instructions: "Linked to Example as @person.",
          revision: 5,
        }),
      ),
    ],
  });

  const rendered = await harness.mount();
  const onChange = attribute(rendered, "onChange")[0];
  onChange({ target: { value: "  hunter2  " } });
  const filled = harness.render();
  const preventDefaults = [];
  await attribute(filled, "onSubmit")[0]({
    preventDefault: () => preventDefaults.push("prevented"),
  });
  const completed = harness.render();

  assert.deepEqual(preventDefaults, ["prevented"], "the form never navigates");
  assert.deepEqual(
    harness.calls.filter((call) => call[0] === "submit"),
    [["submit", DEVICE_LINK_INPUT_KINDS.password, 4, "hunter2"]],
    "the submitted revision is the frame's own, and the value is trimmed",
  );
  assert.deepEqual(attribute(completed, "data-device-link-step"), [DEVICE_LINK_STEPS.completed]);
  assert.ok(stringify(completed).includes("deviceLink.linked"));
  assert.deepEqual(harness.completions, [DEVICE_LINK_STEPS.completed]);
  assert.ok(
    !stringify(completed).includes("hunter2"),
    "the secret is dropped from the card once the host has it",
  );
});

test("DeviceLinkPanel admits only one submit before React commits the loading state", async () => {
  const inputFrame = wireFrame({
    step: DEVICE_LINK_STEPS.inputRequired,
    input_kind: DEVICE_LINK_INPUT_KINDS.code,
    secret_label: "Login code",
    revision: 4,
  });
  let releaseSubmit;
  const pendingSubmit = new Promise((resolve) => {
    releaseSubmit = resolve;
  });
  const harness = createHarness({
    startResponses: [response(inputFrame)],
    pollResponses: [response(inputFrame)],
    submitResponses: [pendingSubmit],
  });

  const rendered = await harness.mount();
  attribute(rendered, "onChange")[0]({ target: { value: "12345" } });
  const filled = harness.render();
  const submit = attribute(filled, "onSubmit")[0];

  const first = submit({ preventDefault() {} });
  const replay = submit({ preventDefault() {} });
  await tick();

  assert.equal(
    harness.calls.filter((call) => call[0] === "submit").length,
    1,
    "two events in the same render must not consume the one-time code twice",
  );

  releaseSubmit(
    response(
      wireFrame({
        step: DEVICE_LINK_STEPS.completed,
        instructions: "Linked.",
        revision: 5,
      }),
    ),
  );
  await Promise.all([first, replay]);
});

test("DeviceLinkPanel restarts in the alternate mode from the QR step", async () => {
  const harness = createHarness({
    startResponses: [
      response(alternateOffered({ qr_payload: "scheme://login?token=AAAA" })),
      response(
        alternateOffered({
          step: DEVICE_LINK_STEPS.inputRequired,
          input_kind: DEVICE_LINK_INPUT_KINDS.identifier,
          secret_label: "Phone number",
          revision: 1,
        }),
        { flowId: "flow-2" },
      ),
    ],
    pollResponses: [response(alternateOffered({ qr_payload: "scheme://login?token=AAAA" }))],
  });

  const rendered = await harness.mount();
  const switchNode = attribute(rendered, "data-device-link-target-mode");
  assert.deepEqual(switchNode, [DEVICE_LINK_MODES.alternate]);

  attribute(rendered, "onClick")[0]();
  harness.render();
  await tick();
  harness.render();
  await tick();
  const phoneView = harness.render();

  assert.deepEqual(
    harness.calls.filter((call) => call[0] === "start" || call[0] === "cancel"),
    [
      ["start", DEVICE_LINK_MODES.default],
      ["cancel", "flow-1"],
      ["start", DEVICE_LINK_MODES.alternate],
    ],
    "the abandoned flow is cancelled before the alternate one starts",
  );
  assert.deepEqual(attribute(phoneView, "data-device-link-mode"), [DEVICE_LINK_MODES.alternate]);
  assert.deepEqual(attribute(phoneView, "data-device-link-input-kind"), [
    DEVICE_LINK_INPUT_KINDS.identifier,
  ]);
  // And the switch now offers the way back.
  assert.deepEqual(attribute(phoneView, "data-device-link-target-mode"), [
    DEVICE_LINK_MODES.default,
  ]);
});

// The card is shared by every device-link vendor, so the ceremony it paints
// has to come off the wire. A vendor that declares one path answers
// `UnsupportedMode` to a switch — an error the user cannot retry out of — so
// an unstated `alternate_available` must render no switch at all.
test("DeviceLinkPanel offers no mode switch to a vendor that declares no second path", async () => {
  const harness = createHarness({
    startResponses: [response(wireFrame({ qr_payload: "scheme://login?token=AAAA" }))],
    pollResponses: [response(wireFrame({ qr_payload: "scheme://login?token=AAAA" }))],
  });

  const rendered = await harness.mount();

  assert.deepEqual(attribute(rendered, "data-device-link-step"), [DEVICE_LINK_STEPS.display]);
  assert.deepEqual(
    attribute(rendered, "data-device-link-target-mode"),
    [],
    "a QR-only vendor gets no switch",
  );
  assert.ok(!stringify(rendered).includes("device-link-mode-switch"));
});

test("DeviceLinkPanel labels the mode switch from the recipe, for the path it moves to", async () => {
  const labelled = {
    alternate_available: true,
    default_mode_label: "Scan a code",
    alternate_mode_label: "Use my phone number",
  };
  const harness = createHarness({
    startResponses: [
      response(wireFrame({ qr_payload: "scheme://login?token=AAAA", ...labelled })),
      response(
        wireFrame({
          step: DEVICE_LINK_STEPS.inputRequired,
          input_kind: DEVICE_LINK_INPUT_KINDS.identifier,
          secret_label: "Phone number",
          mode: DEVICE_LINK_MODES.alternate,
          ...labelled,
        }),
        { flowId: "flow-2" },
      ),
    ],
    pollResponses: [
      response(wireFrame({ qr_payload: "scheme://login?token=AAAA", ...labelled })),
    ],
  });

  const onDefault = await harness.mount();
  assert.ok(
    stringify(onDefault).includes("Use my phone number"),
    "on the primary path the switch is named for the fallback it moves to",
  );
  assert.ok(
    !stringify(onDefault).includes("deviceLink.useAlternate"),
    "a recipe label wins over the generic fallback copy",
  );

  attribute(onDefault, "onClick")[0]();
  harness.render();
  await tick();
  harness.render();
  await tick();
  const onAlternate = harness.render();

  assert.ok(
    stringify(onAlternate).includes("Scan a code"),
    "and on the fallback path it is named for the primary one",
  );

  // Without labels the card falls back to generic host copy — never to one
  // vendor's ceremony.
  const unlabelled = createHarness({
    startResponses: [response(alternateOffered({ qr_payload: "scheme://login?token=AAAA" }))],
    pollResponses: [response(alternateOffered({ qr_payload: "scheme://login?token=AAAA" }))],
  });
  const generic = stringify(await unlabelled.mount());
  assert.ok(generic.includes("deviceLink.useAlternate"));
  assert.ok(!generic.includes("phone"), "the shared fallback copy names no vendor's ceremony");
});

// Bumping the attempt alone left `mode` on whatever had just failed, so a
// vendor that rejects the alternate path re-failed on every "start again"
// with no way back to the path that works.
test("DeviceLinkPanel returns to the vendor's primary path when the user starts again", async () => {
  const harness = createHarness({
    startResponses: [
      response(alternateOffered({ qr_payload: "scheme://login?token=AAAA" })),
      response(
        alternateOffered({
          step: DEVICE_LINK_STEPS.failed,
          instructions: "That path did not work for this account.",
          error_code: "invalid_input",
          restartable: true,
          mode: DEVICE_LINK_MODES.alternate,
        }),
        { flowId: "flow-2" },
      ),
      response(alternateOffered({ qr_payload: "scheme://login?token=RETRY" }), {
        flowId: "flow-3",
      }),
    ],
    pollResponses: [response(alternateOffered({ qr_payload: "scheme://login?token=AAAA" }))],
  });

  // Switch to the alternate path, which fails.
  const rendered = await harness.mount();
  attribute(rendered, "onClick")[0]();
  harness.render();
  await tick();
  harness.render();
  await tick();
  const failedView = harness.render();
  assert.deepEqual(attribute(failedView, "data-device-link-mode"), [DEVICE_LINK_MODES.alternate]);
  assert.ok(stringify(failedView).includes("deviceLink.startAgain"));

  // "Start again" — the only affordance on a failed frame.
  attribute(failedView, "onClick")[0]();
  harness.render();
  await tick();
  harness.render();
  await tick();
  const retried = harness.render();

  assert.deepEqual(
    harness.calls.filter((call) => call[0] === "start"),
    [
      ["start", DEVICE_LINK_MODES.default],
      ["start", DEVICE_LINK_MODES.alternate],
      ["start", DEVICE_LINK_MODES.default],
    ],
    "start again re-runs the primary path, not the one that just failed",
  );
  assert.deepEqual(attribute(retried, "data-device-link-mode"), [DEVICE_LINK_MODES.default]);
  assert.deepEqual(attribute(retried, "data-device-link-step"), [DEVICE_LINK_STEPS.display]);
});

// `display_kind` says what the payload IS. A link rendered as a QR is a
// picture of a URL nobody can scan from the device they are linking, and a
// scannable token rendered as a clickable link invites the browser to open
// the vendor's login token.
test("DeviceLinkPanel renders the affordance the frame's display kind asks for", async () => {
  const cases = [
    {
      displayKind: DEVICE_LINK_DISPLAY_KINDS.link,
      expectQr: false,
      expectOpen: true,
      why: "a link payload is opened, not scanned",
    },
    {
      displayKind: DEVICE_LINK_DISPLAY_KINDS.qrCode,
      expectQr: true,
      expectOpen: false,
      why: "a scannable payload is not offered to this browser as a link",
    },
    {
      displayKind: undefined,
      expectQr: true,
      expectOpen: true,
      // An older host states no kind; the card must paint what it always did.
      why: "an unstated display kind keeps both affordances",
    },
  ];

  for (const { displayKind, expectQr, expectOpen, why } of cases) {
    const frame = wireFrame({
      qr_payload: "scheme://login?token=AAAA",
      ...(displayKind ? { display_kind: displayKind } : {}),
    });
    const harness = createHarness({
      startResponses: [response(frame)],
      pollResponses: [response(frame)],
    });

    const payload = harness.payloadPanelProps(await harness.mount());

    assert.equal(payload.payload, "scheme://login?token=AAAA", why);
    assert.equal(payload.showQr, expectQr, why);
    assert.equal(Boolean(payload.labels.open), expectOpen, why);
  }
});

// A re-render — a refresh, a second tab, a re-opened settings pane — must
// REJOIN the live link. Beginning another one invalidates the payload the
// user is mid-scan on and debits the begin budget for a restart nobody asked
// for.
test("DeviceLinkPanel resumes the flow it was handed, and drops the hint once that flow is abandoned", async () => {
  const live = deviceLinkFrameFromWire(
    alternateOffered({ flow_id: "flow-live", qr_payload: "scheme://login?token=AAAA" }),
  );
  const harness = createHarness({
    startResponses: [
      response(alternateOffered({ qr_payload: "scheme://login?token=AAAA" }), {
        flowId: "flow-live",
      }),
      response(
        alternateOffered({
          step: DEVICE_LINK_STEPS.inputRequired,
          input_kind: DEVICE_LINK_INPUT_KINDS.identifier,
          secret_label: "Phone number",
        }),
        { flowId: "flow-2" },
      ),
    ],
    pollResponses: [
      response(alternateOffered({ qr_payload: "scheme://login?token=AAAA" }), {
        flowId: "flow-live",
      }),
    ],
  });

  const rendered = await harness.mount({ initialFrame: live });

  assert.equal(
    harness.requests.find(([kind]) => kind === "start")[1].resumeFlowId,
    "flow-live",
    "the card rejoins the link it arrived holding",
  );

  // Switching paths deliberately abandons that flow; resuming the link the
  // card just cancelled would undo the switch.
  attribute(rendered, "onClick")[0]();
  harness.render();
  await tick();
  harness.render();
  await tick();
  harness.render({ initialFrame: live });

  assert.deepEqual(
    harness.requests
      .filter(([kind]) => kind === "start")
      .map(([, request]) => request.resumeFlowId),
    ["flow-live", ""],
    "an abandoned flow is not resumed",
  );
});

test("DeviceLinkPanel ignores a poll response carrying a stale step revision", async () => {
  const harness = createHarness({
    startResponses: [response(wireFrame({ qr_payload: "scheme://login?token=AAAA", revision: 7 }))],
    pollResponses: [
      // An overlapping poll resolves late with the frame the card has already
      // moved past. Adopting it would walk the user backwards.
      response(wireFrame({ qr_payload: "scheme://login?token=STALE", revision: 6 })),
      response(wireFrame({ qr_payload: "scheme://login?token=FRESH", revision: 8 })),
    ],
  });

  const rendered = await harness.mount();
  assert.equal(
    harness.payloadPanelProps(rendered).payload,
    "scheme://login?token=AAAA",
  );

  await harness.fireTimers(3000);
  const afterStale = harness.render();
  assert.equal(
    harness.payloadPanelProps(afterStale).payload,
    "scheme://login?token=AAAA",
    "the stale-revision frame is discarded",
  );

  await harness.fireTimers(3000);
  const afterFresh = harness.render();
  assert.equal(
    harness.payloadPanelProps(afterFresh).payload,
    "scheme://login?token=FRESH",
    "a newer revision is adopted",
  );
});

test("DeviceLinkPanel stops polling on every terminal step", async () => {
  for (const terminal of [
    wireFrame({ step: DEVICE_LINK_STEPS.completed, revision: 9 }),
    wireFrame({ step: DEVICE_LINK_STEPS.failed, error_code: "declined", revision: 9 }),
  ]) {
    const harness = createHarness({
      startResponses: [response(wireFrame({ qr_payload: "scheme://login?token=AAAA" }))],
      pollResponses: [response(terminal)],
    });

    await harness.mount();
    assert.ok(harness.timers.active.size > 0, "a live flow polls");

    await harness.fireTimers(3000);
    harness.render();

    assert.equal(
      harness.timers.active.size,
      0,
      `a card left open on ${terminal.step} must hold no timer`,
    );
  }
});

test("DeviceLinkPanel renders the ADR's device-confirmation control on completion", async () => {
  const harness = createHarness({
    startResponses: [response(wireFrame({ qr_payload: "scheme://login?token=AAAA" }))],
    pollResponses: [
      response(
        wireFrame({
          step: DEVICE_LINK_STEPS.completed,
          revision: 9,
          // The resolved account identity rides its own slot. It used to
          // borrow `code`, which left the card unable to tell "read this code
          // to your phone" from "this is who you linked as".
          vendor_user_ref: "+15550000000",
        }),
      ),
    ],
  });

  await harness.mount();
  await harness.fireTimers(3000);
  const text = stringify(harness.render());

  // The whole point of the control (ADR "The one control that is possible"):
  // a user cannot check that the code they scanned came from IronClaw, but
  // they CAN check that exactly one new device appeared, just now.
  assert.ok(
    text.includes("device-link-confirm-device"),
    "a completed link must ask the user to confirm the device count",
  );
  assert.ok(
    text.includes("device-link-account"),
    "the resolved account must be shown, or there is nothing to check against",
  );
  assert.ok(
    text.includes("deviceLink.revokeHint"),
    "the revoke path must stay on screen beside the confirmation ask",
  );

  // …and the account line is genuinely driven by the resolved identity: a
  // completion that carried none must not render an empty "Linked as" claim.
  // A leftover `code` is NOT that identity — it is a short code the vendor
  // issued for the user to read, and showing it as the linked account is the
  // claim this control exists to make checkable.
  const withoutAccount = createHarness({
    startResponses: [response(wireFrame({ qr_payload: "scheme://login?token=AAAA" }))],
    pollResponses: [
      response(wireFrame({ step: DEVICE_LINK_STEPS.completed, revision: 9, code: "AB-CD-12" })),
    ],
  });
  await withoutAccount.mount();
  await withoutAccount.fireTimers(3000);
  const bare = stringify(withoutAccount.render());
  assert.ok(
    bare.includes("device-link-confirm-device"),
    "the confirmation ask does not depend on the account line",
  );
  assert.ok(
    !bare.includes("device-link-account"),
    "no resolved account means no account line",
  );
  assert.ok(
    !bare.includes("AB-CD-12"),
    "a vendor-issued code is not an account identity and must not be shown as one",
  );
});

test("DeviceLinkPanel offers 'start again' on a restartable failure and refuses to on a terminal one", async () => {
  const restartable = createHarness({
    startResponses: [
      response(
        wireFrame({
          step: DEVICE_LINK_STEPS.failed,
          instructions: "Linking did not finish.",
          error_code: "unknown_flow",
          restartable: true,
        }),
      ),
      response(wireFrame({ qr_payload: "scheme://login?token=RETRY" }, { flowId: "flow-2" })),
    ],
  });

  const failedView = await restartable.mount();
  assert.ok(stringify(failedView).includes("deviceLink.startAgain"));
  assert.ok(
    stringify(failedView).includes("deviceLink.error.unknown_flow"),
    "the typed code adds the actionable line",
  );

  attribute(failedView, "onClick")[0]();
  restartable.render();
  await tick();
  restartable.render();
  await tick();
  restartable.render();
  assert.deepEqual(
    restartable.calls.filter((call) => call[0] === "start").length,
    2,
    "start again re-runs the flow rather than dead-ending",
  );

  // An account that can never be linked must not offer a retry forever.
  const terminal = createHarness({
    startResponses: [
      response(
        wireFrame({
          step: DEVICE_LINK_STEPS.failed,
          instructions: "Linking cannot be completed for this account.",
          error_code: "account_unavailable",
        }),
      ),
    ],
  });
  const terminalView = await terminal.mount();
  assert.ok(!stringify(terminalView).includes("deviceLink.startAgain"));
  assert.ok(stringify(terminalView).includes("deviceLink.cannotRetry"));

  const conflicted = createHarness({
    startResponses: [
      response(
        wireFrame({
          step: DEVICE_LINK_STEPS.failed,
          instructions: "This account is already connected elsewhere.",
          error_code: "identity_conflict",
          restartable: false,
        }),
      ),
    ],
  });
  const conflictView = await conflicted.mount();
  assert.ok(
    stringify(conflictView).includes("deviceLink.error.identity_conflict"),
    "an existing owner is explained separately from an ineligible vendor account",
  );
  assert.ok(!stringify(conflictView).includes("deviceLink.startAgain"));
});

test("DeviceLinkPanel surfaces a failed start as a retryable error rather than a blank card", async () => {
  const harness = createHarness({
    startResponses: [{ __reject: { payload: { error: "device link unavailable" } } }],
  });

  const rendered = await harness.mount();

  assert.ok(stringify(rendered).includes("device link unavailable"));
  assert.ok(stringify(rendered).includes("deviceLink.startAgain"));
  assert.equal(harness.timers.active.size, 0, "a card with no flow polls nothing");
});

// A card opened outside a run — the Extensions configure modal — has no
// invocation to carry in. `start` mints one server-side, and the flow's scope
// is stored with it; `scope_matches` is exact equality, so a follow-up call
// that omits it re-derives a different scope and the host answers
// `invalid_request`. Before the response carried `invocation_id`, that link
// could be started and then never advanced.
test("DeviceLinkPanel carries the host-minted invocation into every follow-up call", async () => {
  const harness = createHarness({
    startResponses: [
      { ...response(wireFrame()), invocation_id: "inv-minted-by-host" },
    ],
    pollResponses: [
      {
        ...response(
          wireFrame({
            step: DEVICE_LINK_STEPS.inputRequired,
            input_kind: DEVICE_LINK_INPUT_KINDS.code,
            revision: 2,
          }),
        ),
        invocation_id: "inv-minted-by-host",
      },
    ],
    submitResponses: [
      {
        ...response(wireFrame({ step: DEVICE_LINK_STEPS.completed, revision: 3 })),
        invocation_id: "inv-minted-by-host",
      },
    ],
  });

  // No `invocationId` prop: this is the modal, not a chat gate.
  await harness.mount();
  const started = harness.requests.find(([kind]) => kind === "start")[1];
  assert.ok(!started.invocationId, "start had none to send");

  await harness.fireTimers(3000);
  const polled = harness.requests.find(([kind]) => kind === "poll")[1];
  assert.equal(polled.invocationId, "inv-minted-by-host");
});
