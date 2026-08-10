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
  "automations.notificationChannels.webOnlyHelper":
    "Notifications stay in the web app",
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { NotificationChannelsPanel };`;
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

function t(key) {
  return COPY[key] || key;
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

function createHarness({ saveNotificationChannels = async () => {}, isLoading = false, isSaving = false, error = null, saveError = null } = {}) {
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
    !scalars.includes("Notifications stay in the web app"),
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

test("NotificationChannelsPanel shows the web-app-only helper text once every channel is unchecked", () => {
  const harness = createHarness();
  let rendered = harness.render({
    targets: [target("slack-alpha")],
    channels: [channel("slack-alpha")],
  });
  assert.ok(
    !collectScalars(rendered).includes("Notifications stay in the web app"),
    "helper text must not show while a channel is still selected",
  );

  const [checkbox] = nativeProps(rendered, "input");
  checkbox.onChange({ target: { checked: false } });

  rendered = harness.render({
    targets: [target("slack-alpha")],
    channels: [channel("slack-alpha")],
  });
  assert.ok(
    collectScalars(rendered).includes("Notifications stay in the web app"),
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
