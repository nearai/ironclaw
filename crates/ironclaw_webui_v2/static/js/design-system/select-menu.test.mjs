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

function collectObjects(root) {
  const objects = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (
        value &&
        typeof value === "object" &&
        !Array.isArray(value) &&
        !Array.isArray(value.strings)
      ) {
        objects.push(value);
      }
    }
  });
  return objects;
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

function keyEvent(key) {
  return {
    key,
    preventDefaultCalls: 0,
    preventDefault() {
      this.preventDefaultCalls += 1;
    },
  };
}

function optionClasses(root) {
  return collectScalars(root).filter(
    (value) => typeof value === "string" && value.includes("flex w-full items-center")
  );
}

const defaultOptions = [
  { value: "default", label: "Follow global", tone: "neutral" },
  { value: "always_allow", label: "Always allow", tone: "positive" },
  { value: "ask_each_time", label: "Ask each time", tone: "warning" },
  { value: "disabled", label: "Disabled", tone: "danger" },
];

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
        options: defaultOptions,
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
  assert.match(firstValueAfter(rendered, "aria-owns="), /^v2-select-menu-\d+-listbox$/);
  assert.equal(valuesAfter(rendered, "aria-label=").length, 1);
  assert.ok(
    collectScalars(rendered).some(
      (value) => typeof value === "string" && value.includes("v2-canvas-strong")
    )
  );
  assert.match(optionClasses(rendered)[0], /v2-surface-muted/);
  assert.doesNotMatch(optionClasses(rendered)[0], /v2-accent-soft/);
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

test("SelectMenu starts closed arrow navigation from the selected option", () => {
  const changes = [];
  const harness = createHarness();
  const props = { value: "default", onChange: (value) => changes.push(value) };

  firstValueAfter(harness.render(props), "onClick=")();
  let rendered = harness.render(props);
  valuesAfter(rendered, "onMouseEnter=")[3]();
  firstValueAfter(rendered, "onClick=")();

  rendered = harness.render(props);
  firstValueAfter(rendered, "onKeyDown=")(keyEvent("ArrowDown"));
  rendered = harness.render(props);
  firstValueAfter(rendered, "onKeyDown=")(keyEvent("Enter"));

  assert.deepEqual(changes, ["always_allow"]);
});

test("SelectMenu syncs active index when option order changes", () => {
  const changes = [];
  const harness = createHarness();
  const reorderedOptions = [
    defaultOptions[2],
    defaultOptions[0],
    defaultOptions[1],
    defaultOptions[3],
  ];
  let rendered = harness.render({
    value: "ask_each_time",
    options: defaultOptions,
    onChange: (value) => changes.push(value),
  });

  firstValueAfter(rendered, "onClick=")();
  rendered = harness.render({
    value: "ask_each_time",
    options: defaultOptions,
    onChange: (value) => changes.push(value),
  });
  assert.match(valuesAfter(rendered, "aria-activedescendant=")[0], /-option-2$/);

  harness.render({
    value: "ask_each_time",
    options: reorderedOptions,
    onChange: (value) => changes.push(value),
  });
  rendered = harness.render({
    value: "ask_each_time",
    options: reorderedOptions,
    onChange: (value) => changes.push(value),
  });
  assert.match(valuesAfter(rendered, "aria-activedescendant=")[0], /-option-0$/);

  firstValueAfter(rendered, "onKeyDown=")(keyEvent("ArrowDown"));
  rendered = harness.render({
    value: "ask_each_time",
    options: reorderedOptions,
    onChange: (value) => changes.push(value),
  });
  firstValueAfter(rendered, "onKeyDown=")(keyEvent("Enter"));

  assert.deepEqual(changes, ["default"]);
});

test("SelectMenu only intercepts Escape while open", () => {
  const harness = createHarness();
  const closedEscape = keyEvent("Escape");

  firstValueAfter(harness.render(), "onKeyDown=")(closedEscape);
  assert.equal(closedEscape.preventDefaultCalls, 0);

  firstValueAfter(harness.render(), "onClick=")();
  const opened = harness.render();
  const openEscape = keyEvent("Escape");
  firstValueAfter(opened, "onKeyDown=")(openEscape);

  assert.equal(openEscape.preventDefaultCalls, 1);
  assert.equal(firstValueAfter(harness.render(), "aria-expanded="), "false");
});

test("SelectMenu exposes the active descendant and restores trigger focus on close", () => {
  const harness = createHarness();
  const focusCalls = [];
  const props = { onChange: () => {} };

  let rendered = harness.render(props);
  harness.state.refs[1].current = {
    focus() {
      focusCalls.push("focus");
    },
  };

  firstValueAfter(rendered, "onClick=")();
  rendered = harness.render(props);
  assert.match(valuesAfter(rendered, "aria-activedescendant=")[0], /-option-0$/);

  valuesAfter(rendered, "onClick=")[2]();
  harness.render({ value: "always_allow", ...props });

  assert.deepEqual(focusCalls, ["focus"]);
});

test("SelectMenu does not restore trigger focus after outside click close", () => {
  const harness = createHarness();
  const focusCalls = [];
  const props = { onChange: () => {} };

  let rendered = harness.render(props);
  harness.state.refs[1].current = {
    focus() {
      focusCalls.push("focus");
    },
  };

  firstValueAfter(rendered, "onClick=")();
  rendered = harness.render(props);
  assert.equal(firstValueAfter(rendered, "aria-expanded="), "true");

  harness.state.listeners.mousedown({ target: {} });
  rendered = harness.render(props);

  assert.equal(firstValueAfter(rendered, "aria-expanded="), "false");
  assert.deepEqual(focusCalls, []);
});

test("SelectMenu only passes safe root attributes through rest props", () => {
  const harness = createHarness();
  const rendered = harness.render({
    id: "permission-root",
    "data-testid": "permission-select",
    title: "Permission select",
    onPointerDown() {},
  });
  const passthroughProps = collectObjects(rendered).find(
    (value) => value["data-testid"] === "permission-select"
  );

  assert.equal(passthroughProps.id, "permission-root");
  assert.equal(passthroughProps["data-testid"], "permission-select");
  assert.equal(passthroughProps.title, "Permission select");
  assert.equal("onPointerDown" in passthroughProps, false);
});

test("SelectMenu ignores disabled trigger clicks", () => {
  const harness = createHarness();
  const rendered = harness.render({ disabled: true });

  firstValueAfter(rendered, "onClick=")();
  assert.equal(harness.state.hooks[0], false);
});
