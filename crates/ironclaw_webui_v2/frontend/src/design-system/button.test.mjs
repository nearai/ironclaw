import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

// Load button.js into a vm with its imports stripped, exposing Button and
// letting the test supply the `html`/`cn`/`Spinner` primitives so it can
// inspect the produced template tree.
function buttonSourceForTest() {
  const source = readFileSync(new URL("./button.ts", import.meta.url), "utf8");
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
    lines.push(line.replace("export function Button", "function Button"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { Button };`;
}

function renderButton(props) {
  const spinnerMarker = function Spinner() {
    return { __spinner: true };
  };
  const context = {
    globalThis: {},
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    cn: (...args) => args.filter(Boolean).join(" "),
    Spinner: spinnerMarker,
  };
  vm.runInNewContext(buttonSourceForTest(), context);
  return { tree: context.globalThis.__testExports.Button(props), spinnerMarker };
}

// True if the Spinner component reference appears anywhere in the produced tree.
function treeIncludesComponent(node, component, seen = new Set()) {
  if (node === component) return true;
  if (!node || typeof node !== "object" || seen.has(node)) return false;
  seen.add(node);
  const children = Array.isArray(node) ? node : Object.values(node);
  return children.some((child) => treeIncludesComponent(child, component, seen));
}

// The value passed to `<prop>=${...}` on the element vnode (the first vnode
// whose preceding string ends with `<prop>=`).
function propValue(node, prop) {
  const pattern = new RegExp(`${prop}=\\s*$`);
  for (let index = 0; index < node.strings.length; index += 1) {
    if (pattern.test(node.strings[index])) return node.values[index];
  }
  return undefined;
}

function forwardedProps(node) {
  return node.values.find((value) => value && typeof value === "object" && "onClick" in value);
}

for (const variant of ["primary", "secondary"]) {
  test(`Button loading (${variant}) renders a spinner, disables, and sets aria-busy`, () => {
    const { tree, spinnerMarker } = renderButton({
      variant,
      loading: true,
      children: "Connect",
    });
    assert.ok(treeIncludesComponent(tree, spinnerMarker), "loading shows the spinner");
    assert.equal(propValue(tree, "disabled"), true, "loading disables the button");
    assert.equal(propValue(tree, "aria-busy"), true, "loading sets aria-busy");
  });

  test(`Button idle (${variant}) has no spinner and is enabled`, () => {
    const { tree, spinnerMarker } = renderButton({
      variant,
      loading: false,
      children: "Connect",
    });
    assert.ok(!treeIncludesComponent(tree, spinnerMarker), "no spinner when idle");
    assert.equal(propValue(tree, "disabled"), false, "enabled when idle");
    assert.equal(propValue(tree, "aria-busy"), undefined, "no aria-busy when idle");
  });

  test(`Button loading anchor (${variant}) blocks clicks and marks itself disabled`, () => {
    let clicked = false;
    let prevented = false;
    let stopped = false;
    const { tree } = renderButton({
      as: "a",
      href: "https://example.com/auth",
      variant,
      loading: true,
      onClick: () => {
        clicked = true;
      },
      children: "Connect",
    });

    const props = forwardedProps(tree);
    assert.ok(props, "anchor props are forwarded");
    props.onClick({
      preventDefault: () => {
        prevented = true;
      },
      stopPropagation: () => {
        stopped = true;
      },
    });

    assert.equal(clicked, false, "disabled anchor must not invoke caller onClick");
    assert.equal(prevented, true, "disabled anchor prevents navigation");
    assert.equal(stopped, true, "disabled anchor stops bubbling");
    assert.equal(propValue(tree, "aria-disabled"), true);
    assert.equal(propValue(tree, "tabIndex"), -1);
  });
}

test("Button disabled prop still disables without loading", () => {
  const { tree, spinnerMarker } = renderButton({
    variant: "secondary",
    disabled: true,
    children: "Save",
  });
  assert.equal(propValue(tree, "disabled"), true);
  assert.ok(!treeIncludesComponent(tree, spinnerMarker), "disabled-but-not-loading shows no spinner");
});
