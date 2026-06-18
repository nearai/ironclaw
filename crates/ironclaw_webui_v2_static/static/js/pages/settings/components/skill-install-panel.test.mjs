import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

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
};

function skillInstallPanelSourceForTest() {
  const source = readFileSync(new URL("./skill-install-panel.js", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { SkillInstallPanel };`;
}

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function componentProps(rendered, component) {
  const props = [];
  for (let index = 0; index < rendered.values.length; index += 1) {
    if (rendered.values[index] !== component) continue;
    const current = {};
    for (let propIndex = index + 1; propIndex < rendered.values.length; propIndex += 1) {
      const name = rendered.strings[propIndex]?.match(/([A-Za-z][A-Za-z0-9-]*)=\s*$/)?.[1];
      if (name) {
        current[name] = rendered.values[propIndex];
      } else if (typeof rendered.values[propIndex] === "function") {
        break;
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
    globalThis: {},
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
  vm.runInNewContext(skillInstallPanelSourceForTest(), context);

  return {
    Button,
    FormField,
    Input,
    Textarea,
    installs,
    render() {
      cursor = 0;
      return context.globalThis.__testExports.SkillInstallPanel({
        isInstalling: false,
        onInstall: async (payload) => {
          installs.push(payload);
          return onInstall(payload);
        },
      });
    },
  };
}

test("SkillInstallPanel clears required-field errors when fields become valid", async () => {
  const harness = createHarness();
  let rendered = harness.render();

  await componentProps(rendered, harness.Button)[0].onClick();
  assert.deepEqual(harness.installs, []);

  rendered = harness.render();
  let fields = componentProps(rendered, harness.FormField);
  assert.equal(fields[0].error, "Skill name is required.");
  assert.equal(fields[1].error, "SKILL.md content is required.");

  componentProps(rendered, harness.Input)[0].onInput({
    currentTarget: { value: "summarizer" },
  });

  rendered = harness.render();
  fields = componentProps(rendered, harness.FormField);
  assert.equal(fields[0].error, "");
  assert.equal(fields[1].error, "SKILL.md content is required.");

  componentProps(rendered, harness.Textarea)[0].onInput({
    currentTarget: { value: "---\nname: summarizer\n---\nSummarize documents." },
  });

  rendered = harness.render();
  fields = componentProps(rendered, harness.FormField);
  assert.equal(fields[0].error, "");
  assert.equal(fields[1].error, "");

  await componentProps(rendered, harness.Button)[0].onClick();
  assert.deepEqual(JSON.parse(JSON.stringify(harness.installs)), [
    {
      name: "summarizer",
      content: "---\nname: summarizer\n---\nSummarize documents.",
    },
  ]);
});
