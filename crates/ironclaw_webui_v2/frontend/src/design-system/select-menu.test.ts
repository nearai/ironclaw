// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../test-support/vm-module-harness";

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
    if (
      node.props &&
      typeof node.props === "object" &&
      !Array.isArray(node.props)
    ) {
      objects.push(node.props);
    }
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

function firstObjectWith(root, key) {
  const value = collectObjects(root).find((candidate) => key in candidate);
  assert.notEqual(value, undefined, `expected object with ${key}`);
  return value;
}

function dependenciesChanged(previous, next) {
  if (!previous || !next) return true;
  if (previous.length !== next.length) return true;
  return next.some((value, index) => !Object.is(value, previous[index]));
}

function keyEvent(key) {
  return {
    key,
    preventDefaultCalls: 0,
    stopPropagationCalls: 0,
    preventDefault() {
      this.preventDefaultCalls += 1;
    },
    stopPropagation() {
      this.stopPropagationCalls += 1;
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
    effectCursor: 0,
    effects: [],
    hookCursor: 0,
    hooks: [],
    listenerAdds: 0,
    listenerRemoves: 0,
    listeners: {},
    refCursor: 0,
    refs: [],
  };
  const context = {
    React: {
      useEffect(effect, dependencies) {
        const index = state.effectCursor;
        state.effectCursor += 1;
        const nextDependencies = dependencies ? Array.from(dependencies) : null;
        const previous = state.effects[index];
        if (
          previous &&
          nextDependencies &&
          !dependenciesChanged(previous.dependencies, nextDependencies)
        ) {
          return undefined;
        }
        if (previous?.cleanup) previous.cleanup();
        const cleanup = effect();
        state.effects[index] = {
          dependencies: nextDependencies,
          cleanup: typeof cleanup === "function" ? cleanup : null,
        };
        return undefined;
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
        state.listenerAdds += 1;
        state.listeners[type] = listener;
      },
      removeEventListener(type, listener) {
        state.listenerRemoves += 1;
        if (state.listeners[type] === listener) delete state.listeners[type];
      },
    },
    html,
    cn,
    Icon,
  };
  const exports = runVmModuleForTest(
    "./select-menu.tsx",
    ["SelectMenu"],
    context,
    import.meta.url
  );
  return {
    state,
    SelectMenu: exports.SelectMenu,
    render(props = {}) {
      state.effectCursor = 0;
      state.hookCursor = 0;
      state.refCursor = 0;
      return exports.SelectMenu({
        value: "default",
        options: defaultOptions,
        "aria-label": "Tool permission",
        ...props,
      });
    },
    renderPair(firstProps = {}, secondProps = {}) {
      state.effectCursor = 0;
      state.hookCursor = 0;
      state.refCursor = 0;
      return [
        exports.SelectMenu({
          value: "default",
          options: defaultOptions,
          "aria-label": "First permission",
          ...firstProps,
        }),
        exports.SelectMenu({
          value: "default",
          options: defaultOptions,
          "aria-label": "Second permission",
          ...secondProps,
        }),
      ];
    },
    unmount() {
      for (const effect of state.effects) {
        if (effect?.cleanup) effect.cleanup();
      }
      state.effects = [];
      state.effectCursor = 0;
    },
  };
}

test("SelectMenu renders a closed custom trigger with the selected label", () => {
  const harness = createHarness();
  const rendered = harness.render();

  assert.match(collectTemplateText(rendered), /aria-haspopup="listbox"/);
  assert.equal(firstValueAfter(rendered, "aria-expanded="), "false");
  assert.equal(collectObjects(rendered).some((value) => "aria-owns" in value), false);
  assert.equal(collectObjects(rendered).some((value) => "aria-controls" in value), false);
  assert.equal(
    collectObjects(rendered).some((value) => "aria-activedescendant" in value),
    false
  );
  assert.ok(collectScalars(rendered).includes("Follow global"));
  assert.doesNotMatch(collectTemplateText(rendered), /<select/);
  assert.doesNotMatch(collectTemplateText(rendered), /role="listbox"/);
});

test("SelectMenu opens, selects an option, and closes after selection", () => {
  const changes = [];
  const harness = createHarness();

  firstValueAfter(harness.render({ onChange: (...args) => changes.push(args) }), "onClick=")();
  let rendered = harness.render({ onChange: (...args) => changes.push(args) });
  assert.equal(firstValueAfter(rendered, "aria-expanded="), "true");
  assert.match(collectTemplateText(rendered), /role="listbox"/);
  const listboxProps = firstObjectWith(rendered, "aria-controls");
  assert.equal("aria-owns" in listboxProps, false);
  assert.match(listboxProps["aria-controls"], /^v2-select-menu-\d+-listbox$/);
  assert.match(listboxProps["aria-activedescendant"], /^v2-select-menu-\d+-option-0$/);
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
  assert.deepEqual(changes, [["always_allow"]]);

  rendered = harness.render({ value: "always_allow", onChange: (...args) => changes.push(args) });
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
  assert.match(
    firstObjectWith(rendered, "aria-activedescendant")["aria-activedescendant"],
    /-option-2$/
  );

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
  assert.match(
    firstObjectWith(rendered, "aria-activedescendant")["aria-activedescendant"],
    /-option-0$/
  );

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
  assert.equal(closedEscape.stopPropagationCalls, 0);

  firstValueAfter(harness.render(), "onClick=")();
  const opened = harness.render();
  const openEscape = keyEvent("Escape");
  firstValueAfter(opened, "onKeyDown=")(openEscape);

  assert.equal(openEscape.preventDefaultCalls, 1);
  assert.equal(openEscape.stopPropagationCalls, 1);
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
  assert.match(
    firstObjectWith(rendered, "aria-activedescendant")["aria-activedescendant"],
    /-option-0$/
  );

  valuesAfter(rendered, "onClick=")[2]();
  harness.render({ value: "always_allow", ...props });

  assert.deepEqual(focusCalls, ["focus"]);
});

test("SelectMenu does not restore trigger focus after outside click close", () => {
  const harness = createHarness();
  const focusCalls = [];
  const props = { onChange: () => {} };

  let rendered = harness.render(props);
  harness.state.refs[0].current = {
    contains() {
      return false;
    },
    isConnected: true,
  };
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

test("SelectMenu shares and removes the document listener across concurrent menus", () => {
  const harness = createHarness();
  let rendered = harness.renderPair();
  harness.state.refs[0].current = { contains: () => false, isConnected: true };
  harness.state.refs[6].current = { contains: () => false, isConnected: true };
  const [firstTriggerClick, secondTriggerClick] = valuesAfter(rendered, "onClick=");

  firstTriggerClick();
  rendered = harness.renderPair();
  assert.equal(harness.state.listenerAdds, 1);

  secondTriggerClick();
  rendered = harness.renderPair();
  assert.equal(harness.state.listenerAdds, 1);
  assert.deepEqual(valuesAfter(rendered, "aria-expanded="), ["true", "true"]);

  harness.state.listeners.mousedown({ target: {} });
  rendered = harness.renderPair();

  assert.deepEqual(valuesAfter(rendered, "aria-expanded="), ["false", "false"]);
  assert.equal(harness.state.listenerRemoves, 1);
  assert.equal(harness.state.listeners.mousedown, undefined);
});

test("SelectMenu removes the document listener when unmounted while open", () => {
  const harness = createHarness();
  const rendered = harness.render();
  harness.state.refs[0].current = { contains: () => false, isConnected: true };

  firstValueAfter(rendered, "onClick=")();
  harness.render();
  assert.equal(harness.state.listenerAdds, 1);

  harness.unmount();

  assert.equal(harness.state.listenerRemoves, 1);
  assert.equal(harness.state.listeners.mousedown, undefined);
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

test("SelectMenu does not open when every option is disabled", () => {
  const harness = createHarness();
  const disabledOptions = defaultOptions.map((option) => ({ ...option, disabled: true }));
  const rendered = harness.render({ options: disabledOptions });
  const arrowDown = keyEvent("ArrowDown");

  assert.equal(firstValueAfter(rendered, "disabled="), true);
  firstValueAfter(rendered, "onClick=")();
  firstValueAfter(rendered, "onKeyDown=")(arrowDown);

  assert.equal(harness.state.hooks[0], false);
  assert.equal(arrowDown.preventDefaultCalls, 0);
});
