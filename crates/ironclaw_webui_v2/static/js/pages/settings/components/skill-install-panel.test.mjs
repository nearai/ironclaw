import assert from "node:assert/strict";
import test from "node:test";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness.test.mjs";

const COPY = {
  "skills.content": "SKILL.md content",
  "skills.contentHint": "Use the full SKILL.md frontmatter and prompt content.",
  "skills.contentPlaceholder": "---\nname: example\n---\n",
  "skills.contentRequired": "SKILL.md content is required.",
  "skills.import": "Import skill",
  "skills.importDesc": "Paste SKILL.md content to add a user-mounted skill.",
  "skills.install": "Import",
  "skills.installFailed": "Import failed.",
  "skills.installedSuccess": "Added skill \"{name}\"",
  "skills.installing": "Importing...",
  "skills.name": "Skill name",
  "skills.namePlaceholder": "skill-name",
  "skills.nameRequired": "Skill name is required.",
  "skills.shareWithAllUsers": "Share with all users",
  "skills.shareWithAllUsersHint":
    "Installs as an admin-managed shared skill (the name gets the shared- prefix). Visible to every user, read-only for them.",
};

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

function componentProps(rendered, component, allComponents) {
  const props = [];
  for (let index = 0; index < rendered.values.length; index += 1) {
    if (rendered.values[index] !== component) continue;
    const current = {};
    for (let propIndex = index + 1; propIndex < rendered.values.length; propIndex += 1) {
      if (allComponents.has(rendered.values[propIndex])) break;
      const name = rendered.strings[propIndex]?.match(/([A-Za-z][A-Za-z0-9-]*)=\s*$/)?.[1];
      if (name) {
        current[name] = rendered.values[propIndex];
      }
    }
    props.push(current);
  }
  return props;
}

function createHarness({ onInstall = async () => ({ success: true }) } = {}) {
  const state = [];
  let cursor = 0;

  function Button() {}
  function Card() {}
  function FormField() {}
  function Icon() {}
  function Input() {}
  function Textarea() {}

  const React = {
    useCallback(fn) {
      return fn;
    },
    useState(initial) {
      const index = cursor;
      cursor += 1;
      if (!(index in state)) state[index] = initial;
      return [
        state[index],
        (next) => {
          state[index] = typeof next === "function" ? next(state[index]) : next;
        },
      ];
    },
  };

  const installs = [];
  const context = {
    Boolean,
    Button,
    Card,
    FormField,
    Icon,
    Input,
    React,
    Textarea,
    html,
    useT: () => (key, values = {}) => {
      let value = COPY[key] || key;
      for (const [name, replacement] of Object.entries(values)) {
        value = value.replace(`{${name}}`, replacement);
      }
      return value;
    },
  };
  const exports = runVmModuleForTest(
    "./skill-install-panel.js",
    ["SkillInstallPanel"],
    context,
    import.meta.url
  );
  const allComponents = new Set([Button, Card, FormField, Icon, Input, Textarea]);

  return {
    Button,
    FormField,
    Input,
    Textarea,
    installs,
    props(rendered, component) {
      return componentProps(rendered, component, allComponents);
    },
    render({ isInstalling = false, isAdmin = false } = {}) {
      cursor = 0;
      return exports.SkillInstallPanel({
        isInstalling,
        isAdmin,
        onInstall: async (payload) => {
          installs.push(payload);
          return onInstall(payload);
        },
      });
    },
  };
}

// Find the value bound to `name=` anywhere in the rendered template tree —
// used for raw-element props (the shared checkbox is a plain <input>, not a
// design-system component).
function rawPropByName(root, name) {
  const matches = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !Array.isArray(node.values)) return;
    for (let index = 0; index < node.values.length; index += 1) {
      const bound = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9-]*)=\s*$/)?.[1];
      if (bound === name) matches.push(node.values[index]);
    }
  });
  return matches;
}

test("SkillInstallPanel clears required-field errors when fields become valid", async () => {
  const harness = createHarness();
  let rendered = harness.render();

  await harness.props(rendered, harness.Button)[0].onClick();
  assert.deepEqual(harness.installs, []);

  rendered = harness.render();
  let fields = harness.props(rendered, harness.FormField);
  let inputs = harness.props(rendered, harness.Input);
  let textareas = harness.props(rendered, harness.Textarea);
  assert.equal(fields[0].error, "Skill name is required.");
  assert.equal(fields[1].error, "SKILL.md content is required.");
  assert.equal(inputs[0].error, true);
  assert.equal(inputs[0]["aria-invalid"], "true");
  assert.equal(textareas[0].error, true);
  assert.equal(textareas[0]["aria-invalid"], "true");

  inputs[0].onInput({
    currentTarget: { value: "summarizer" },
  });

  rendered = harness.render();
  fields = harness.props(rendered, harness.FormField);
  inputs = harness.props(rendered, harness.Input);
  textareas = harness.props(rendered, harness.Textarea);
  assert.equal(fields[0].error, "");
  assert.equal(fields[1].error, "SKILL.md content is required.");
  assert.equal(inputs[0].error, false);
  assert.equal(inputs[0]["aria-invalid"], undefined);
  assert.equal(textareas[0].error, true);
  assert.equal(textareas[0]["aria-invalid"], "true");

  textareas[0].onInput({
    currentTarget: { value: "---\nname: summarizer\n---\nSummarize documents." },
  });

  rendered = harness.render();
  fields = harness.props(rendered, harness.FormField);
  inputs = harness.props(rendered, harness.Input);
  textareas = harness.props(rendered, harness.Textarea);
  assert.equal(fields[0].error, "");
  assert.equal(fields[1].error, "");
  assert.equal(inputs[0].error, false);
  assert.equal(inputs[0]["aria-invalid"], undefined);
  assert.equal(textareas[0].error, false);
  assert.equal(textareas[0]["aria-invalid"], undefined);

  await harness.props(rendered, harness.Button)[0].onClick();
  assert.deepEqual(JSON.parse(JSON.stringify(harness.installs)), [
    {
      name: "summarizer",
      content: "---\nname: summarizer\n---\nSummarize documents.",
    },
  ]);

  rendered = harness.render();
  assert.equal(harness.props(rendered, harness.Input)[0].value, "");
  assert.equal(harness.props(rendered, harness.Textarea)[0].value, "");
  assert.ok(collectScalars(rendered).includes('Added skill "summarizer"'));
});

test("SkillInstallPanel displays install failures without resetting entered values", async () => {
  const harness = createHarness({
    onInstall: async () => ({ success: false, message: "Skill already exists." }),
  });
  let rendered = harness.render();

  harness.props(rendered, harness.Input)[0].onInput({
    currentTarget: { value: "summarizer" },
  });
  rendered = harness.render();
  harness.props(rendered, harness.Textarea)[0].onInput({
    currentTarget: { value: "---\nname: summarizer\n---\nSummarize documents." },
  });
  rendered = harness.render();

  await harness.props(rendered, harness.Button)[0].onClick();

  rendered = harness.render();
  assert.deepEqual(JSON.parse(JSON.stringify(harness.installs)), [
    {
      name: "summarizer",
      content: "---\nname: summarizer\n---\nSummarize documents.",
    },
  ]);
  assert.equal(harness.props(rendered, harness.Input)[0].value, "summarizer");
  assert.equal(
    harness.props(rendered, harness.Textarea)[0].value,
    "---\nname: summarizer\n---\nSummarize documents."
  );
  assert.ok(collectScalars(rendered).includes("Skill already exists."));
});

test("SkillInstallPanel disables submit and changes label while installing", () => {
  const harness = createHarness();
  const rendered = harness.render({ isInstalling: true });
  const button = harness.props(rendered, harness.Button)[0];

  assert.equal(button.disabled, true);
  assert.ok(collectScalars(rendered).includes("Importing..."));
});

// #5459 P4: the share toggle is admin-only, and checking it must put
// `shared: true` in the install payload — this pins the buildPayload
// passthrough (a payload builder that drops the flag regresses silently).
test("SkillInstallPanel sends shared flag only when admin checks the toggle", async () => {
  const harness = createHarness();

  // Non-admin render: no share toggle at all.
  let rendered = harness.render();
  assert.equal(rawPropByName(rendered, "onChange").length, 0);
  assert.ok(!collectScalars(rendered).includes("Share with all users"));

  // Admin render: toggle present, unchecked installs stay private
  // (payload omits the flag entirely for pre-#5459 request-shape parity).
  rendered = harness.render({ isAdmin: true });
  assert.ok(collectScalars(rendered).includes("Share with all users"));
  harness.props(rendered, harness.Input)[0].onInput({
    currentTarget: { value: "team-brief" },
  });
  rendered = harness.render({ isAdmin: true });
  harness.props(rendered, harness.Textarea)[0].onInput({
    currentTarget: { value: "---\nname: team-brief\n---\nBody." },
  });
  rendered = harness.render({ isAdmin: true });
  await harness.props(rendered, harness.Button)[0].onClick();
  assert.deepEqual(JSON.parse(JSON.stringify(harness.installs)), [
    {
      name: "team-brief",
      content: "---\nname: team-brief\n---\nBody.",
    },
  ]);

  // Check the toggle: the next install carries shared: true.
  rendered = harness.render({ isAdmin: true });
  const onChange = rawPropByName(rendered, "onChange")[0];
  assert.equal(typeof onChange, "function");
  onChange({ currentTarget: { checked: true } });
  rendered = harness.render({ isAdmin: true });
  harness.props(rendered, harness.Input)[0].onInput({
    currentTarget: { value: "team-brief" },
  });
  rendered = harness.render({ isAdmin: true });
  harness.props(rendered, harness.Textarea)[0].onInput({
    currentTarget: { value: "---\nname: team-brief\n---\nBody." },
  });
  rendered = harness.render({ isAdmin: true });
  await harness.props(rendered, harness.Button)[0].onClick();
  assert.deepEqual(JSON.parse(JSON.stringify(harness.installs.at(-1))), {
    name: "team-brief",
    content: "---\nname: team-brief\n---\nBody.",
    shared: true,
  });

  // Success resets the toggle so the next install defaults back to private.
  rendered = harness.render({ isAdmin: true });
  assert.equal(rawPropByName(rendered, "checked")[0], false);
});
