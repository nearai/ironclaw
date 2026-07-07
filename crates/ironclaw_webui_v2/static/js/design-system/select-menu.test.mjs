import assert from "node:assert/strict";
import test from "node:test";

import { runVmModuleForTest } from "../test-support/vm-module-harness.test.mjs";

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function cn(...classes) {
  return classes.flat().filter(Boolean).join(" ");
}

function Icon() {}

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

function collectTemplateText(root) {
  const text = [];
  visit(root, (node) => {
    if (Array.isArray(node.strings)) text.push(...node.strings);
  });
  return text.join("");
}

function valuesAfter(root, fragment) {
  const values = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !Array.isArray(node.values)) return;
    node.strings.forEach((part, index) => {
      if (part.includes(fragment)) values.push(node.values[index]);
    });
  });
  return values;
}

function firstValueAfter(root, fragment) {
  const [value] = valuesAfter(root, fragment);
  assert.notEqual(value, undefined, `expected template fragment ${fragment}`);
  return value;
}

function createHarness() {
  const state = {
    hookCursor: 0,
    hooks: [],
    listeners: {},
    refCursor: 0,
    refs: [],
  };
  const context = {
    React: {
      useEffect(effect) {
        return effect();
      },
      useRef(initialValue) {
        const index = state.refCursor;
        state.refCursor += 1;
        if (!state.refs[index]) state.refs[index] = { current: initialValue };
        return state.refs[index];
      },
      useState(initialValue) {
        const index = state.hookCursor;
        state.hookCursor += 1;
        if (!(index in state.hooks)) {
          state.hooks[index] =
            typeof initialValue === "function" ? initialValue() : initialValue;
        }
        return [
          state.hooks[index],
          (nextValue) => {
            state.hooks[index] =
              typeof nextValue === "function" ? nextValue(state.hooks[index]) : nextValue;
          },
        ];
      },
    },
    document: {
      addEventListener(type, listener) {
        state.listeners[type] = listener;
      },
      removeEventListener(type, listener) {
        if (state.listeners[type] === listener) delete state.listeners[type];
      },
    },
    html,
    cn,
    Icon,
  };
  const exports = runVmModuleForTest(
    "./select-menu.js",
    ["SelectMenu"],
    context,
    import.meta.url
  );
  return {
    state,
    SelectMenu: exports.SelectMenu,
    render(props = {}) {
      state.hookCursor = 0;
      state.refCursor = 0;
      return exports.SelectMenu({
        value: "default",
        options: [
          { value: "default", label: "Follow global", tone: "neutral" },
          { value: "always_allow", label: "Always allow", tone: "positive" },
          { value: "ask_each_time", label: "Ask each time", tone: "warning" },
          { value: "disabled", label: "Disabled", tone: "danger" },
        ],
        "aria-label": "Tool permission",
        ...props,
      });
    },
  };
}

test("SelectMenu renders a closed custom trigger with the selected label", () => {
  const harness = createHarness();
  const rendered = harness.render();

  assert.match(collectTemplateText(rendered), /aria-haspopup="listbox"/);
  assert.equal(firstValueAfter(rendered, "aria-expanded="), "false");
  assert.ok(collectScalars(rendered).includes("Follow global"));
  assert.doesNotMatch(collectTemplateText(rendered), /<select/);
  assert.doesNotMatch(collectTemplateText(rendered), /role="listbox"/);
});

test("SelectMenu opens, selects an option, and closes after selection", () => {
  const changes = [];
  const harness = createHarness();

  firstValueAfter(harness.render({ onChange: (value) => changes.push(value) }), "onClick=")();
  let rendered = harness.render({ onChange: (value) => changes.push(value) });
  assert.equal(firstValueAfter(rendered, "aria-expanded="), "true");
  assert.match(collectTemplateText(rendered), /role="listbox"/);
  assert.ok(
    collectScalars(rendered).some(
      (value) => typeof value === "string" && value.includes("v2-canvas-strong")
    )
  );
  assert.ok(collectScalars(rendered).includes("Always allow"));

  valuesAfter(rendered, "onClick=")[2]();
  assert.deepEqual(changes, ["always_allow"]);

  rendered = harness.render({ value: "always_allow", onChange: (value) => changes.push(value) });
  assert.equal(firstValueAfter(rendered, "aria-expanded="), "false");
});

test("SelectMenu supports keyboard navigation and Enter selection", () => {
  const changes = [];
  const harness = createHarness();
  const keyDown = firstValueAfter(
    harness.render({ onChange: (value) => changes.push(value) }),
    "onKeyDown="
  );
  const event = (key) => ({ key, preventDefault() {} });

  keyDown(event("ArrowDown"));
  const opened = harness.render({ onChange: (value) => changes.push(value) });
  assert.equal(firstValueAfter(opened, "aria-expanded="), "true");

  firstValueAfter(opened, "onKeyDown=")(event("Enter"));
  assert.deepEqual(changes, ["always_allow"]);
});

test("SelectMenu ignores disabled trigger clicks", () => {
  const harness = createHarness();
  const rendered = harness.render({ disabled: true });

  firstValueAfter(rendered, "onClick=")();
  assert.equal(harness.state.hooks[0], false);
});
