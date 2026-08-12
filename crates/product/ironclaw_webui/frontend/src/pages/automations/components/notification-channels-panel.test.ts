// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import { mergeNotificationChannelRows } from "../hooks/useNotificationChannels";

const COPY = {
  "automations.notificationChannels.eyebrow": "Notification channels",
  "automations.notificationChannels.title": "Where notifications are sent",
  "automations.notificationChannels.explainer":
    "Choose which connected channels receive approval prompts, auth prompts, and run-failure notices.",
  "automations.notificationChannels.empty": "No connected channels yet.",
  "automations.notificationChannels.noSelectionHelper":
    "No notification channel is selected — approval prompts, auth prompts, and failure notices won't be delivered anywhere.",
  "automations.notificationChannels.devicePush.deviceHeading": "This browser",
  "automations.notificationChannels.devicePush.checking":
    "Checking this browser's notification support…",
  "automations.notificationChannels.devicePush.unsupported":
    "Push notifications aren't available in this browser.",
  "automations.notificationChannels.devicePush.permissionDenied":
    "Notifications are blocked for this site. Allow them in your browser's site settings, then try again.",
  "automations.notificationChannels.devicePush.notEnrolled":
    "This browser isn't receiving notifications yet.",
  "automations.notificationChannels.devicePush.enrolled":
    "This browser receives notifications.",
  "automations.notificationChannels.devicePush.enroll":
    "Enable notifications in this browser",
  "automations.notificationChannels.devicePush.unenroll": "Disable in this browser",
  "automations.notificationChannels.devicePush.deviceCount": "Enrolled browsers: {count}",
  "automations.notificationChannels.devicePush.actionFailed":
    "Couldn't update this browser's notification enrollment. Please try again.",
  "automations.notificationChannels.devicePush.enrolledOtherAccount":
    "This browser is enrolled for a different account. You can enable it for this account too.",
  "automations.notificationChannels.devicePush.enableForAccount": "Enable for this account",
  "automations.notificationChannels.devicePush.statusFailed":
    "Couldn't load this account's enrollment status. Try reloading the page.",
  "automations.notificationChannels.save": "Save",
  "automations.notificationChannels.saved": "Saved",
  "automations.notificationChannels.saveFailed":
    "Couldn't save your notification channels. Please try again.",
  "automations.notificationChannels.pill.ready": "Ready",
  "automations.notificationChannels.pill.unavailable": "Unavailable",
  "automations.notificationChannels.loadFailedEditingDisabled":
    "Editing is off until your saved channels load — saving now would replace them with an incomplete list.",
  "automations.error.loadFailed": "Unable to load automations",
};

function sourceForTest() {
  const source = readFileSync(
    new URL("./notification-channels-panel.tsx", import.meta.url),
    "utf8",
  );
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { NotificationChannelsPanel, DevicePushBlock, SessionChannelRow };`;
}

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  if (Array.isArray(node.values)) {
    for (const value of node.values) visit(value, fn);
  }
}

function collectScalars(root) {
  const scalars = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
        scalars.push(value);
      }
    }
  });
  return scalars;
}

function componentProps(root, component) {
  const props = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (let index = 0; index < node.values.length; index += 1) {
      if (node.values[index] !== component) continue;
      const current = {};
      for (let propIndex = index + 1; propIndex < node.values.length; propIndex += 1) {
        const name = node.strings[propIndex]?.match(/([A-Za-z][A-Za-z0-9-]*)=\s*$/)?.[1];
        if (name) current[name] = node.values[propIndex];
      }
      props.push(current);
    }
  });
  return props;
}

function nativeProps(root, tagName) {
  const props = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !node.strings.join("").includes(`<${tagName}`)) return;
    const current = {};
    node.strings.forEach((part, index) => {
      const name = part.match(/([A-Za-z][A-Za-z0-9-]*)=\s*$/)?.[1];
      if (name) current[name] = node.values[index];
    });
    props.push(current);
  });
  return props;
}

function t(key, params = {}) {
  const text = COPY[key] || key;
  return text.replace(/\{([a-zA-Z0-9_]+)\}/g, (match, name) =>
    name in params ? String(params[name]) : match,
  );
}

function channel(targetId, { displayName, description, status = "available" } = {}) {
  const isAvailable = status === "available";
  return {
    target_id: targetId,
    status,
    option: isAvailable
      ? {
          target: {
            target_id: targetId,
            display_name: displayName || targetId,
            description: description || null,
            channel: "slack",
          },
          capabilities: { final_replies: false, gate_prompts: true, auth_prompts: true },
        }
      : null,
  };
}

function target(targetId, { displayName, description } = {}) {
  return {
    target: {
      target_id: targetId,
      display_name: displayName || targetId,
      description: description || null,
      channel: "slack",
    },
    capabilities: { final_replies: false, gate_prompts: true, auth_prompts: true },
  };
}

/** Drives the real `mergeNotificationChannelRows` (not a test-local copy) so
 * this panel test exercises the same row shape production renders —
 * including a stored id absent from the catalog (the Task 8 "unavailable,
 * not dropped" contract). */
function mergeRows(targets, channels) {
  const rows = mergeNotificationChannelRows(targets, channels);
  const selected = rows.filter((row) => row.selected).map((row) => row.target_id);
  return { rows, selected };
}

function createHarness({ saveNotificationChannels = async () => {}, isLoading = false, isSaving = false, error = null, saveError = null, devicePush = null } = {}) {
  const hookValues = [];
  const effectDeps = [];
  let hookCursor = 0;

  function Badge() {}
  function Button() {}
  function Icon() {}
  function Panel() {}

  const React = {
    useEffect(effect, deps) {
      const index = hookCursor;
      hookCursor += 1;
      const previous = effectDeps[index];
      const changed =
        !previous ||
        !deps ||
        deps.length !== previous.length ||
        deps.some((value, depIndex) => value !== previous[depIndex]);
      if (changed) {
        effectDeps[index] = deps ? [...deps] : deps;
        effect();
      }
    },
    useState(initial) {
      const index = hookCursor;
      hookCursor += 1;
      if (!(index in hookValues)) {
        hookValues[index] = typeof initial === "function" ? initial() : initial;
      }
      return [
        hookValues[index],
        (next) => {
          hookValues[index] =
            typeof next === "function" ? next(hookValues[index]) : next;
        },
      ];
    },
    useRef(initial) {
      const index = hookCursor;
      hookCursor += 1;
      if (!(index in hookValues)) hookValues[index] = { current: initial };
      return hookValues[index];
    },
    useMemo(factory, deps) {
      const index = hookCursor;
      hookCursor += 1;
      const previous = effectDeps[index];
      const changed =
        !previous ||
        !deps ||
        deps.length !== previous.length ||
        deps.some((value, depIndex) => value !== previous[depIndex]);
      if (changed) {
        effectDeps[index] = deps ? [...deps] : deps;
        hookValues[index] = factory();
      }
      return hookValues[index];
    },
  };

  const context = {
    globalThis: {},
    Badge,
    Button,
    Icon,
    Panel,
    React,
    TextEncoder,
    cn: (...parts) => parts.filter(Boolean).join(" "),
    html,
    useT: () => t,
    getSessionChannelExtensionId: () => "session-channel",
    useDevicePush: () =>
      devicePush || {
        browser: { state: "not-enrolled" },
        subscriptionCount: 0,
        vapidPublicKey: "test-vapid-key",
        isStatusLoading: false,
        statusError: null,
        isBusy: false,
        actionError: null,
        enroll: async () => ({ state: "enrolled" }),
        unenroll: async () => ({ state: "not-enrolled" }),
      },
  };

  vm.runInNewContext(sourceForTest(), context);
  const exports = context.globalThis.__testExports;
  return {
    Button,
    exports,
    render({ targets = [], channels = [] } = {}) {
      const { rows, selected } = mergeRows(targets, channels);
      hookCursor = 0;
      return exports.NotificationChannelsPanel({
        channelsState: {
          rows,
          selectedIds: selected,
          isLoading,
          isSaving,
          error,
          saveError,
          saveNotificationChannels,
        },
      });
    },
  };
}

test("NotificationChannelsPanel shows a load error instead of claiming there are no channels", () => {
  const harness = createHarness({ error: new Error("catalog unavailable") });
  const rendered = harness.render();
  const scalars = collectScalars(rendered);
  assert.ok(scalars.includes("Unable to load automations"));
  assert.ok(
    !scalars.includes("No connected channels yet."),
    "a backend outage must not look like a truthful empty catalog"
  );
});

test("NotificationChannelsPanel locks editing after a failed read so Save cannot become a destructive full replace", () => {
  const saved = [];
  const harness = createHarness({
    error: new Error("notification-channels read failed"),
    saveNotificationChannels: async (targetIds) => {
      saved.push(targetIds);
    },
  });

  // The destructive shape: the notification-channels GET failed while the
  // targets GET succeeded, so the hook yields channels=[] => selectedIds=[]
  // and every genuinely-stored channel renders UNCHECKED. Toggling one row
  // would make the draft dirty and Save posts a FULL REPLACE — silently
  // dropping every stored channel the user never got to see. Editing must
  // stay locked until the read actually succeeded (tool-evidence UI rule:
  // render backend evidence or a disabled state, never a guess).
  let rendered = harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [],
  });

  const checkboxes = nativeProps(rendered, "input");
  assert.equal(checkboxes.length, 2);
  for (const checkbox of checkboxes) {
    assert.equal(
      checkbox.disabled,
      true,
      "a failed read must disable every row, not just render an alert above them",
    );
  }

  // Even if a click lands (a stale render, a keyboard activation), the toggle
  // must not stage a draft the user could then save.
  checkboxes[0].onChange({ target: { checked: true } });
  rendered = harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [],
  });
  const [saveButton] = componentProps(rendered, harness.Button);
  assert.equal(saveButton.disabled, true, "Save must stay disabled while the read is failed");

  saveButton.onClick();
  assert.deepEqual(saved, [], "a failed read must never reach the full-replace write");

  const scalars = collectScalars(rendered);
  assert.ok(
    scalars.includes(
      "Editing is off until your saved channels load — saving now would replace them with an incomplete list.",
    ),
    "the user must be told why the panel is disabled, not left with an inert form",
  );
  assert.ok(
    !scalars.includes("No notification channel is selected — approval prompts, auth prompts, and failure notices won't be delivered anywhere."),
    "the empty-selection helper is a claim about the stored set — a failed read must not assert it",
  );
});

test("NotificationChannelsPanel renders one checkbox row per channel, checked to match the stored set", () => {
  const harness = createHarness();
  const rendered = harness.render({
    targets: [target("slack-alpha", { displayName: "Slack — #ops" }), target("slack-beta")],
    channels: [channel("slack-alpha", { displayName: "Slack — #ops" })],
  });

  const checkboxes = nativeProps(rendered, "input");
  assert.equal(checkboxes.length, 2);
  const alpha = checkboxes.find((box) => box.checked === true);
  assert.ok(alpha, "the stored channel's checkbox should be checked");
  const scalars = collectScalars(rendered);
  assert.ok(scalars.includes("Slack — #ops"));
});

test("NotificationChannelsPanel stages a toggle without saving until Save is clicked", () => {
  const saved = [];
  const harness = createHarness({
    saveNotificationChannels: async (targetIds) => {
      saved.push(targetIds);
    },
  });

  let rendered = harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [channel("slack-alpha")],
  });
  const [saveButton] = componentProps(rendered, harness.Button);
  assert.equal(saveButton.disabled, true, "no change yet, so Save starts disabled");

  const checkboxes = nativeProps(rendered, "input");
  const betaCheckbox = checkboxes[1];
  betaCheckbox.onChange({ target: { checked: true } });

  rendered = harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [channel("slack-alpha")],
  });
  const [saveButtonAfterToggle] = componentProps(rendered, harness.Button);
  assert.equal(saveButtonAfterToggle.disabled, false, "toggling stages a dirty, savable draft");
  assert.deepEqual(saved, [], "toggling alone must not call the mutation");
});

test("NotificationChannelsPanel Save posts the full-replace target_ids array", () => {
  const saved = [];
  const harness = createHarness({
    saveNotificationChannels: async (targetIds) => {
      saved.push(targetIds);
    },
  });

  let rendered = harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [channel("slack-alpha")],
  });
  const checkboxes = nativeProps(rendered, "input");
  checkboxes[1].onChange({ target: { checked: true } });

  rendered = harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [channel("slack-alpha")],
  });
  const [saveButton] = componentProps(rendered, harness.Button);
  saveButton.onClick();

  assert.equal(saved.length, 1);
  assert.deepEqual(
    [...saved[0]].sort(),
    ["slack-alpha", "slack-beta"],
    "Save must post every currently-checked target_id, not just the toggled one",
  );
});

test("NotificationChannelsPanel shows the no-channel-selected helper text once every channel is unchecked", () => {
  const harness = createHarness();
  let rendered = harness.render({
    targets: [target("slack-alpha")],
    channels: [channel("slack-alpha")],
  });
  assert.ok(
    !collectScalars(rendered).includes("No notification channel is selected — approval prompts, auth prompts, and failure notices won't be delivered anywhere."),
    "helper text must not show while a channel is still selected",
  );

  const [checkbox] = nativeProps(rendered, "input");
  checkbox.onChange({ target: { checked: false } });

  rendered = harness.render({
    targets: [target("slack-alpha")],
    channels: [channel("slack-alpha")],
  });
  assert.ok(
    collectScalars(rendered).includes("No notification channel is selected — approval prompts, auth prompts, and failure notices won't be delivered anywhere."),
    "unchecking every channel must show the empty-selection helper text",
  );
});

test("NotificationChannelsPanel renders a stored-but-dead channel as an unavailable, still-toggleable row", () => {
  const saved = [];
  const harness = createHarness({
    saveNotificationChannels: async (targetIds) => {
      saved.push(targetIds);
    },
  });

  // "slack-gone" is stored (Task 8's RebornNotificationChannel) but absent
  // from the live target catalog — status unavailable, option: null.
  let rendered = harness.render({
    targets: [target("slack-alpha")],
    channels: [
      channel("slack-alpha"),
      channel("slack-gone", { status: "unavailable" }),
    ],
  });

  assert.ok(
    collectScalars(rendered).includes("Unavailable"),
    "the dead channel's row must carry the unavailable tone, not be dropped",
  );
  const checkboxes = nativeProps(rendered, "input");
  assert.equal(checkboxes.length, 2, "the unavailable channel must still render a row");
  const deadCheckbox = checkboxes[1];
  assert.equal(deadCheckbox.checked, true, "a stored channel starts checked even if unavailable");

  deadCheckbox.onChange({ target: { checked: false } });
  rendered = harness.render({
    targets: [target("slack-alpha")],
    channels: [
      channel("slack-alpha"),
      channel("slack-gone", { status: "unavailable" }),
    ],
  });
  const [saveButton] = componentProps(rendered, harness.Button);
  saveButton.onClick();

  assert.deepEqual(
    [...saved[0]],
    ["slack-alpha"],
    "deselecting the dead channel must drop it from the next full-replace Save",
  );
});

test("NotificationChannelsPanel Save excludes a still-checked dead channel instead of failing the whole set", () => {
  const saved = [];
  const harness = createHarness({
    saveNotificationChannels: async (targetIds) => {
      saved.push(targetIds);
    },
  });

  // "slack-gone" is stored but unresolvable: it stays rendered and checked,
  // but the backend rejects the entire full-replace write on the first
  // unresolvable id — so Save must never send it.
  let rendered = harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [
      channel("slack-alpha"),
      channel("slack-gone", { status: "unavailable" }),
    ],
  });
  const checkboxes = nativeProps(rendered, "input");
  // Keep the dead channel checked; toggle a live one on.
  checkboxes[1].onChange({ target: { checked: true } });

  rendered = harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [
      channel("slack-alpha"),
      channel("slack-gone", { status: "unavailable" }),
    ],
  });
  const [saveButton] = componentProps(rendered, harness.Button);
  saveButton.onClick();

  assert.equal(saved.length, 1);
  assert.deepEqual(
    [...saved[0]].sort(),
    ["slack-alpha", "slack-beta"],
    "the unresolvable id must be excluded from the payload so the save can succeed",
  );
});

test("NotificationChannelsPanel resyncs the draft when the stored set changes", () => {
  const harness = createHarness();
  let rendered = harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [channel("slack-alpha")],
  });
  let checkboxes = nativeProps(rendered, "input");
  assert.equal(checkboxes[0].checked, true);
  assert.equal(checkboxes[1].checked, false);

  // A mid-session stored-set change (a save reconcile or another tab's
  // write) must clobber the draft: checkbox state follows the NEW
  // selectedIds, not the pre-change draft. The shim runs effects during
  // render, so the resync's state update lands on the FOLLOWING render —
  // mirroring React's effect-then-re-render sequence.
  harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [channel("slack-beta")],
  });
  rendered = harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [channel("slack-beta")],
  });
  checkboxes = nativeProps(rendered, "input");
  assert.equal(
    checkboxes[0].checked,
    false,
    "a channel dropped from the stored set must uncheck on resync",
  );
  assert.equal(
    checkboxes[1].checked,
    true,
    "a channel added to the stored set must check on resync",
  );
});

test("NotificationChannelsPanel renders a resolved orphan channel's real display name and status, not the raw target_id", () => {
  const harness = createHarness();

  // "slack-progress-bot" is stored and resolved (option populated, its "true
  // status" is available) but is not part of the live target catalog —
  // reachable e.g. via the model tool. It must render its own identity, not
  // collapse to the raw id the way a truly-dead (option: null) channel does.
  const rendered = harness.render({
    targets: [target("slack-alpha")],
    channels: [
      channel("slack-alpha"),
      channel("slack-progress-bot", { displayName: "Progress Bot" }),
    ],
  });

  const scalars = collectScalars(rendered);
  assert.ok(
    scalars.includes("Progress Bot"),
    "a resolved orphan must render its option's display_name — if the merge regressed to " +
      "the raw-id fallback, this would be absent (the row's own text would read " +
      "\"slack-progress-bot\" instead, and this assertion would fail)",
  );
  // Not asserting the raw id is absent from `scalars` entirely: the harness's
  // JSX shim sweeps up the React `key` prop (`key={row.target_id}`, required
  // and correct) as a scalar too, so that check would be a false positive on
  // the harness, not the component.
  const checkboxes = nativeProps(rendered, "input");
  assert.equal(checkboxes.length, 2);
  assert.equal(
    checkboxes[1].checked,
    true,
    "a resolved orphan is still part of the stored set and starts checked",
  );
});

test("NotificationChannelsPanel does not hardcode a catalog row's badge to Ready when the response reports it unavailable", () => {
  const harness = createHarness();

  // "slack-alpha" is in the live catalog (inherently resolvable) but the
  // notification-channels response reports it unavailable for this caller —
  // the per-channel response status must win over catalog presence.
  const rendered = harness.render({
    targets: [target("slack-alpha")],
    channels: [channel("slack-alpha", { status: "unavailable" })],
  });

  const scalars = collectScalars(rendered);
  assert.ok(
    scalars.includes("Unavailable"),
    "catalog presence must not force the Ready badge when the response says unavailable",
  );
  assert.ok(
    !scalars.includes("Ready"),
    "the Ready badge must not render for a row the response marked unavailable",
  );
});

// ── Session-channel row: the device block under the session channel ────────
// The harness advertises "session-channel" as the deployment's session
// channel id (see `getSessionChannelExtensionId` above); this target's
// `channel` matches it, the way the live catalog row matches the
// `GET /session`-advertised id. No channel name is hardcoded anywhere.

function sessionChannelTarget() {
  return {
    target: {
      target_id: "browser-registration-1",
      display_name: "Web app",
      description: "Browser push notifications to your enrolled devices",
      channel: "session-channel",
    },
    capabilities: { final_replies: true, gate_prompts: true, auth_prompts: true },
  };
}

function fakeDevice(overrides = {}) {
  return {
    browser: { state: "not-enrolled" },
    subscriptionCount: 0,
    vapidPublicKey: "test-vapid-key",
    isStatusLoading: false,
    statusError: null,
    isBusy: false,
    actionError: null,
    enroll: async () => ({ state: "enrolled" }),
    unenroll: async () => ({ state: "not-enrolled" }),
    ...overrides,
  };
}

// The vm harness never invokes child components — they appear in the panel
// tree as (component, props) nodes. So the panel-level tests assert the
// block is MOUNTED with the right props (and only under the session row),
// and the block's own rendering is asserted by calling it directly.

test("NotificationChannelsPanel mounts the session-channel row under the session-channel target only", () => {
  const harness = createHarness();
  const withSessionChannel = harness.render({
    targets: [target("slack-alpha"), sessionChannelTarget()],
    channels: [],
  });
  assert.equal(
    componentProps(withSessionChannel, harness.exports.SessionChannelRow).length,
    1,
    "exactly one session-channel row for the session-channel target",
  );

  const withoutSessionChannel = harness.render({
    targets: [target("slack-alpha"), target("slack-beta")],
    channels: [],
  });
  assert.equal(
    componentProps(withoutSessionChannel, harness.exports.SessionChannelRow).length,
    0,
    "no session-channel row without a matching target — the setup-status query never mounts for deployments without one",
  );
});

test("SessionChannelRow wires the device hook into the block", () => {
  // The session-channel row is the only place the device hook runs; the block
  // stays a pure view. Prove the row passes the hook's state through, so owning
  // the hook here (not at the panel top) didn't sever the block from its data.
  const device = fakeDevice({
    subscriptionCount: 2,
    browser: { state: "enrolled", accountMatch: true },
  });
  const harness = createHarness({ devicePush: device });
  const rendered = harness.exports.SessionChannelRow({
    row: sessionChannelTarget().target,
    isSelected: false,
    isEditingLocked: false,
    onToggle: () => {},
    t,
  });
  const [block] = componentProps(rendered, harness.exports.DevicePushBlock);
  assert.ok(block, "the row renders the device block");
  assert.equal(block.device, device, "the block receives the hook's device state");
});

test("NotificationChannelsPanel toggles the session-channel target into the full-replace save set like any channel", () => {
  const saved = [];
  const harness = createHarness({
    saveNotificationChannels: async (targetIds) => {
      saved.push(targetIds);
    },
  });
  let rendered = harness.render({
    targets: [sessionChannelTarget()],
    channels: [],
  });
  // The session channel's checkbox lives inside SessionChannelRow — a child
  // the vm harness renders as a node without invoking — so drive the panel's
  // toggle through the callback it hands that row (the same one the child's
  // checkbox onChange calls).
  const [sessionRow] = componentProps(rendered, harness.exports.SessionChannelRow);
  assert.ok(sessionRow, "the panel renders the session-channel row");
  assert.equal(sessionRow.isSelected, false);
  sessionRow.onToggle(sessionRow.row.target_id);

  rendered = harness.render({ targets: [sessionChannelTarget()], channels: [] });
  const [saveButton] = componentProps(rendered, harness.Button);
  assert.equal(saveButton.disabled, false, "a staged session-channel toggle enables Save");
  saveButton.onClick();
  assert.equal(saved.length, 1);
  assert.deepEqual(
    [...saved[0]],
    ["browser-registration-1"],
    "Save posts the persisted catalog target id, not the session extension id",
  );
});

test("SessionChannelRow stays pending while account enrollment status loads", () => {
  const harness = createHarness({
    devicePush: fakeDevice({ isStatusLoading: true, subscriptionCount: 0 }),
  });
  const rendered = harness.exports.SessionChannelRow({
    row: sessionChannelTarget().target,
    isSelected: false,
    isEditingLocked: false,
    onToggle: () => {},
    t,
  });
  const [checkbox] = nativeProps(rendered, "input");
  const scalars = collectScalars(rendered);

  assert.equal(checkbox.disabled, true, "pending setup evidence cannot be selected yet");
  assert.ok(
    scalars.includes("Checking this browser's notification support…"),
    "the row must present pending evidence while setup status loads",
  );
  assert.ok(!scalars.includes("Unavailable"), "loading is not evidence of unavailability");
});

test("SessionChannelRow cannot be selected when no browser is enrolled", () => {
  // subscription_count === 0 → nowhere to deliver a push, so the session
  // channel is not selectable and its pill is not "Ready". The nested device
  // block's "Enable notifications in this browser" button is how the user
  // fixes it (covered by the DevicePushBlock tests below).
  const harness = createHarness({ devicePush: fakeDevice({ subscriptionCount: 0 }) });
  const rendered = harness.exports.SessionChannelRow({
    row: sessionChannelTarget().target,
    isSelected: false,
    isEditingLocked: false,
    onToggle: () => {},
    t,
  });
  const [checkbox] = nativeProps(rendered, "input");
  assert.equal(
    checkbox.disabled,
    true,
    "with zero enrolled browsers the session channel's checkbox cannot be selected",
  );
  const scalars = collectScalars(rendered);
  assert.ok(!scalars.includes("Ready"), "an unenrolled session-channel row must not claim it is Ready");
  assert.ok(
    scalars.includes("Unavailable"),
    "the pill reflects that the channel has no enrolled browser to deliver to",
  );
});

test("SessionChannelRow keeps an already-selected channel deselectable with zero enrolled browsers", () => {
  // A browser that unsubscribes after the channel was saved must not leave the
  // checkbox locked ON: the disable applies only to SELECTING (an unchecked
  // row), so a stored selection can always be turned back off.
  const harness = createHarness({ devicePush: fakeDevice({ subscriptionCount: 0 }) });
  const rendered = harness.exports.SessionChannelRow({
    row: sessionChannelTarget().target,
    isSelected: true,
    isEditingLocked: false,
    onToggle: () => {},
    t,
  });
  const [checkbox] = nativeProps(rendered, "input");
  assert.equal(checkbox.checked, true);
  assert.equal(
    checkbox.disabled,
    false,
    "a stored session-channel selection stays deselectable even with zero enrolled browsers",
  );
});

test("SessionChannelRow is selectable and Ready once a browser is enrolled", () => {
  const harness = createHarness({
    devicePush: fakeDevice({
      subscriptionCount: 1,
      browser: { state: "enrolled", accountMatch: true },
    }),
  });
  const rendered = harness.exports.SessionChannelRow({
    row: sessionChannelTarget().target,
    isSelected: false,
    isEditingLocked: false,
    onToggle: () => {},
    t,
  });
  const [checkbox] = nativeProps(rendered, "input");
  assert.equal(checkbox.disabled, false, "an enrolled browser makes the session channel selectable");
  assert.ok(
    collectScalars(rendered).includes("Ready"),
    "an enrolled session-channel row is Ready",
  );
});

test("SessionChannelRow keeps the checkbox disabled while editing is locked, even when enrolled", () => {
  const harness = createHarness({ devicePush: fakeDevice({ subscriptionCount: 2 }) });
  const rendered = harness.exports.SessionChannelRow({
    row: sessionChannelTarget().target,
    isSelected: false,
    isEditingLocked: true,
    onToggle: () => {},
    t,
  });
  const [checkbox] = nativeProps(rendered, "input");
  assert.equal(
    checkbox.disabled,
    true,
    "a locked panel (loading/failed read) disables the session channel's checkbox like every other row",
  );
});

test("DevicePushBlock distinguishes unsupported, denied, not-enrolled, and enrolled browsers", () => {
  const harness = createHarness();
  const block = harness.exports.DevicePushBlock;
  const cases = [
    [
      { state: "unsupported" },
      "Push notifications aren't available in this browser.",
    ],
    [
      { state: "permission-denied" },
      "Notifications are blocked for this site. Allow them in your browser's site settings, then try again.",
    ],
    [
      { state: "not-enrolled" },
      "This browser isn't receiving notifications yet.",
    ],
    [
      { state: "enrolled", endpoint: "https://fcm.googleapis.com/send/x", accountMatch: true },
      "This browser receives notifications.",
    ],
    [
      {
        state: "enrolled-other-account",
        endpoint: "https://fcm.googleapis.com/send/x",
        accountMatch: false,
      },
      "This browser is enrolled for a different account. You can enable it for this account too.",
    ],
    [
      {
        state: "enrolled-unverified",
        endpoint: "https://fcm.googleapis.com/send/x",
        accountMatch: null,
      },
      "This browser receives notifications.",
    ],
  ];
  for (const [browser, expectedCopy] of cases) {
    const rendered = block({
      device: fakeDevice({ browser, subscriptionCount: 2 }),
      t,
    });
    const scalars = collectScalars(rendered);
    assert.ok(scalars.includes("This browser"), "device heading renders");
    assert.ok(
      scalars.includes(expectedCopy),
      `state ${browser.state} must render its copy`,
    );
    assert.ok(
      scalars.includes("Enrolled browsers: 2"),
      "the enrolled-device count is interpolated",
    );
  }
});

test("DevicePushBlock enroll and unenroll buttons call the device hook", () => {
  const harness = createHarness();
  const block = harness.exports.DevicePushBlock;

  const enrollCalls = [];
  const notEnrolled = block({
    device: fakeDevice({
      enroll: async () => {
        enrollCalls.push("enroll");
        return { state: "enrolled" };
      },
    }),
    t,
  });
  const [enrollButton] = componentProps(notEnrolled, harness.Button);
  assert.ok(enrollButton, "the enroll button renders for a not-enrolled browser");
  assert.equal(enrollButton.disabled, false);
  enrollButton.onClick();
  assert.deepEqual(enrollCalls, ["enroll"]);

  const unenrollCalls = [];
  const enrolled = block({
    device: fakeDevice({
      browser: { state: "enrolled", endpoint: "https://fcm.googleapis.com/send/x" },
      unenroll: async () => {
        unenrollCalls.push("unenroll");
        return { state: "not-enrolled" };
      },
    }),
    t,
  });
  const [unenrollButton] = componentProps(enrolled, harness.Button);
  assert.ok(unenrollButton, "the unenroll button renders for an enrolled browser");
  unenrollButton.onClick();
  assert.deepEqual(unenrollCalls, ["unenroll"]);

  const busy = block({
    device: fakeDevice({ isBusy: true }),
    t,
  });
  const [busyButton] = componentProps(busy, harness.Button);
  assert.equal(busyButton.disabled, true, "actions lock while a request is in flight");
});

test("DevicePushBlock surfaces an action failure", () => {
  const harness = createHarness();
  const block = harness.exports.DevicePushBlock;
  const rendered = block({
    device: fakeDevice({ actionError: new Error("nope") }),
    t,
  });
  assert.ok(
    collectScalars(rendered).includes(
      "Couldn't update this browser's notification enrollment. Please try again.",
    ),
  );
});

test("DevicePushBlock offers enable-for-this-account (never disable) for another account's subscription", () => {
  const harness = createHarness();
  const block = harness.exports.DevicePushBlock;

  const enrollCalls = [];
  const unenrollCalls = [];
  const rendered = block({
    device: fakeDevice({
      browser: {
        state: "enrolled-other-account",
        endpoint: "https://fcm.googleapis.com/send/x",
        accountMatch: false,
      },
      enroll: async () => {
        enrollCalls.push("enroll");
        return { state: "enrolled" };
      },
      unenroll: async () => {
        unenrollCalls.push("unenroll");
        return { state: "not-enrolled" };
      },
    }),
    t,
  });
  const buttons = componentProps(rendered, harness.Button);
  assert.equal(
    buttons.length,
    1,
    "exactly one action for another account's subscription — no local disable that would sever it",
  );
  const scalars = collectScalars(rendered);
  assert.ok(scalars.includes("Enable for this account"));
  assert.ok(
    !scalars.includes("Disable in this browser"),
    "the local unsubscribe must not be offered for a subscription this account does not own",
  );
  buttons[0].onClick();
  assert.deepEqual(enrollCalls, ["enroll"], "the action routes through the enroll flow");
  assert.deepEqual(unenrollCalls, []);
});

test("DevicePushBlock offers no actions for an unverified enrollment", () => {
  const harness = createHarness();
  const block = harness.exports.DevicePushBlock;
  const rendered = block({
    device: fakeDevice({
      browser: {
        state: "enrolled-unverified",
        endpoint: "https://fcm.googleapis.com/send/x",
        accountMatch: null,
      },
      statusError: new Error("status query failed"),
    }),
    t,
  });
  assert.equal(
    componentProps(rendered, harness.Button).length,
    0,
    "without account correlation neither enroll nor the destructive disable is offered",
  );
});

test("DevicePushBlock renders the status failure instead of claiming zero enrolled browsers", () => {
  const harness = createHarness();
  const block = harness.exports.DevicePushBlock;
  const rendered = block({
    device: fakeDevice({ statusError: new Error("status query failed") }),
    t,
  });
  const scalars = collectScalars(rendered);
  assert.ok(
    scalars.includes("Couldn't load this account's enrollment status. Try reloading the page."),
    "a failed status query must be announced",
  );
  assert.ok(
    !scalars.includes("Enrolled browsers: 0"),
    "a failed status query must not be rendered as a truthful zero-device count",
  );
});

test("DevicePushBlock disables enroll while the VAPID key is missing", () => {
  // `vapidPublicKey` is "" until the status query resolves (or after it
  // fails). The enroll click would then always throw `vapidPublicKey is
  // required`, so the gate on the key is load-bearing — pin it.
  const harness = createHarness();
  const block = harness.exports.DevicePushBlock;
  const rendered = block({
    device: fakeDevice({ vapidPublicKey: "" }),
    t,
  });
  const [enrollButton] = componentProps(rendered, harness.Button);
  assert.ok(enrollButton, "the enroll button still renders");
  assert.equal(
    enrollButton.disabled,
    true,
    "enroll must stay disabled until the VAPID key is available",
  );
});
