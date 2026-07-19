// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

const COPY = {
  "automations.detail.currentRun": "Current run",
  "automations.detail.emptyDescription": "Select an automation",
  "automations.detail.emptyTitle": "No automation selected",
  "automations.detail.lastCompleted": "Last completed",
  "automations.detail.noCurrentRun": "No current run",
  "automations.detail.noRuns": "No runs",
  "automations.detail.recentRuns": "Recent runs",
  "automations.detail.schedule": "Schedule",
  "automations.detail.successRate": "Success rate",
  "automations.rename.action": "Rename",
  "automations.rename.nameLabel": "Automation name",
  "automations.rename.nameRequired": "Enter a name.",
  "automations.rename.nameTooLong": "Name must be 256 bytes or fewer.",
  "common.cancel": "Cancel",
  "common.delete": "Delete",
  "common.save": "Save",
  "missions.action.pause": "Pause",
  "missions.action.resume": "Resume",
};

function sourceForTest() {
  const source = readFileSync(new URL("./automation-detail-panel.tsx", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { AutomationDetailPanel };`;
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

function automation() {
  return {
    automation_id: "automation-alpha",
    current_run: null,
    display_name: "Daily status",
    has_failed_runs: false,
    has_running_run: false,
    last_run_label: "Never",
    primary_status_label: "Active",
    primary_status_tone: "success",
    recent_runs: [],
    schedule_label: "0 9 * * *",
    state: "active",
    success_rate_label: "100%",
  };
}

function createHarness({ onRenameAutomation = () => {} } = {}) {
  const hookValues = [];
  const effectDeps = [];
  let hookCursor = 0;

  function Button() {}
  function ConfirmDialog() {}
  function EmptyPanel() {}
  function Icon() {}
  function Input() {}
  function Panel() {}
  function RecentRunRow() {}
  function RunDots() {}
  function RunHistorySummary() {}
  function StatusPill() {}

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
      if (!(index in hookValues)) hookValues[index] = initial;
      return [
        hookValues[index],
        (next) => {
          hookValues[index] =
            typeof next === "function" ? next(hookValues[index]) : next;
        },
      ];
    },
  };

  const context = {
    globalThis: {},
    Button,
    ConfirmDialog,
    EmptyPanel,
    Icon,
    Input,
    Panel,
    RecentRunRow,
    React,
    RunDots,
    RunHistorySummary,
    StatusPill,
    TextEncoder,
    cn: (...parts) => parts.filter(Boolean).join(" "),
    html,
    recentRunKey: (run) => run.run_id,
    useNavigate: () => () => {},
    useT: () => t,
  };

  vm.runInNewContext(sourceForTest(), context);
  const exports = context.globalThis.__testExports;
  return {
    Button,
    ConfirmDialog,
    Input,
    exports,
    render(overrides = {}) {
      hookCursor = 0;
      return exports.AutomationDetailPanel({
        automation: automation(),
        onRenameAutomation,
        ...overrides,
      });
    },
  };
}

test("AutomationDetailPanel deletes only after confirming the shared dialog", () => {
  const deletions = [];
  const harness = createHarness();

  let rendered = harness.render({
    onDeleteAutomation: (automationId) => deletions.push(automationId),
  });
  const deleteButton = componentProps(rendered, harness.Button).find(
    (button) => button["aria-label"] === "Delete: Daily status",
  );
  assert.ok(deleteButton, "delete button should render");

  deleteButton.onClick();
  assert.deepEqual(deletions, []);

  rendered = harness.render({
    onDeleteAutomation: (automationId) => deletions.push(automationId),
  });
  const [dialog] = componentProps(rendered, harness.ConfirmDialog);
  assert.equal(dialog.open, true);
  assert.equal(dialog.title, "Delete: Daily status");

  dialog.onConfirm();
  assert.deepEqual(deletions, ["automation-alpha"]);
});

test("AutomationDetailPanel submits a trimmed rename from the inline editor", () => {
  const calls = [];
  const harness = createHarness({
    onRenameAutomation: (payload) => calls.push(payload),
  });

  let rendered = harness.render();
  const editButton = componentProps(rendered, harness.Button).find(
    (button) => button["aria-label"] === "Rename: Daily status",
  );
  assert.ok(editButton, "rename edit button should render");

  editButton.onClick();
  rendered = harness.render();
  const [input] = componentProps(rendered, harness.Input);
  assert.equal(input.value, "Daily status");

  input.onInput({ currentTarget: { value: "  Renamed status  " } });
  rendered = harness.render();
  const [form] = nativeProps(rendered, "form");
  let prevented = false;
  form.onSubmit({
    preventDefault: () => {
      prevented = true;
    },
  });

  assert.equal(prevented, true);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].automationId, "automation-alpha");
  assert.equal(calls[0].name, "Renamed status");
});

test("AutomationDetailPanel keeps the editor open and reports blank names", () => {
  const calls = [];
  const harness = createHarness({
    onRenameAutomation: (payload) => calls.push(payload),
  });

  let rendered = harness.render();
  const editButton = componentProps(rendered, harness.Button).find(
    (button) => button["aria-label"] === "Rename: Daily status",
  );
  editButton.onClick();

  rendered = harness.render();
  const [input] = componentProps(rendered, harness.Input);
  input.onInput({ currentTarget: { value: "   " } });

  rendered = harness.render();
  const [form] = nativeProps(rendered, "form");
  form.onSubmit({ preventDefault: () => {} });

  rendered = harness.render();
  assert.deepEqual(calls, []);
  assert.ok(collectScalars(rendered).includes("Enter a name."));
});

test("AutomationDetailPanel preserves rename drafts across same automation refresh", () => {
  const harness = createHarness();

  let rendered = harness.render();
  const editButton = componentProps(rendered, harness.Button).find(
    (button) => button["aria-label"] === "Rename: Daily status",
  );
  editButton.onClick();

  rendered = harness.render();
  let [input] = componentProps(rendered, harness.Input);
  input.onInput({ currentTarget: { value: "Draft in progress" } });

  const refreshedAutomation = {
    ...automation(),
    display_name: "Server refreshed status",
  };
  rendered = harness.render({ automation: refreshedAutomation });
  rendered = harness.render({ automation: refreshedAutomation });

  [input] = componentProps(rendered, harness.Input);
  assert.equal(input.value, "Draft in progress");
});

// #5886: when the presenter attaches hold_meta_label (active_hold present),
// the detail panel must surface it near the status pill.
test("AutomationDetailPanel renders hold_meta_label when present", () => {
  const harness = createHarness();
  const rendered = harness.render({
    automation: {
      ...automation(),
      primary_status_label: "Waiting for your approval",
      primary_status_tone: "warning",
      hold_meta_label:
        "Paused since Jul 14, 12:51 AM · 3 scheduled occurrences elapsed while held",
    },
  });

  assert.ok(
    collectScalars(rendered).includes(
      "Paused since Jul 14, 12:51 AM · 3 scheduled occurrences elapsed while held",
    ),
    "hold_meta_label text should render in the detail panel",
  );
});

test("AutomationDetailPanel omits hold meta text when hold_meta_label is absent", () => {
  const harness = createHarness();
  const rendered = harness.render();

  assert.ok(
    !collectScalars(rendered).some((value) =>
      typeof value === "string" && value.includes("scheduled occurrences elapsed"),
    ),
    "no hold meta text should render when there is no active hold",
  );
});
