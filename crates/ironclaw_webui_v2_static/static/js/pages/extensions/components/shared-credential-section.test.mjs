import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// Mirrors the source-munging harness used across the extensions component
// tests. The `html` tagged-template stub returns a single flat node
// { strings, values } for the whole component (htm interpolations are captured
// in one array), so FormField / Input markers appear as values mid-array and
// the attributes that follow a marker live in the next `strings` entry.

function stripImports(source) {
  const lines = source.split("\n");
  const out = [];
  let inBlockImport = false;
  for (const line of lines) {
    if (inBlockImport) {
      if (/^\s*\}/.test(line) && /from\s+["']/.test(line)) {
        inBlockImport = false;
      }
      continue;
    }
    if (line.startsWith("import ")) {
      if (line.includes("{") && !line.includes("}")) {
        inBlockImport = true;
      }
      continue;
    }
    out.push(line.replace(/^export function /, "function "));
  }
  return out.join("\n");
}

function sourceForTest() {
  const source = readFileSync(new URL("./shared-credential-section.js", import.meta.url), "utf8");
  return (
    stripImports(source) +
    "\nglobalThis.__testExports = { SharedCredentialSection, FormField, Input };"
  );
}

function makeContext() {
  const React = {
    useState: (initial) => [typeof initial === "function" ? initial() : initial, () => {}],
    useCallback: (fn) => fn,
    useEffect: () => {},
  };
  function html(strings, ...values) {
    return { strings: Array.from(strings), values };
  }
  function useT() {
    return (key) => key;
  }
  function Button() {}
  function Icon() {}
  function FormField() {}
  function Input() {}
  function useSharedCredential() {
    return { setCredential: async () => ({}), isSaving: false };
  }
  return {
    globalThis: {},
    React,
    html,
    useT,
    Button,
    Icon,
    FormField,
    Input,
    useSharedCredential,
  };
}

function render(defaultHandle) {
  const context = makeContext();
  vm.runInNewContext(sourceForTest(), context);
  const { SharedCredentialSection, FormField, Input } = context.globalThis.__testExports;
  const rendered = SharedCredentialSection({ defaultHandle });
  return { rendered, FormField, Input };
}

// ---------------------------------------------------------------------------
// Tests (#5459)
// ---------------------------------------------------------------------------

test("the handle is rendered read-only, not as an editable field", () => {
  const { rendered, FormField, Input } = render("market_data_api_key");
  const { strings, values } = rendered;

  // The fixed handle is shown as plain interpolated context text.
  assert.ok(
    values.includes("market_data_api_key"),
    "the fixed handle should be shown as read-only context text"
  );

  // Exactly one Input (the write-only value) and one FormField (wrapping it) —
  // no editable handle field.
  const inputIndices = values.reduce((acc, v, i) => (v === Input ? [...acc, i] : acc), []);
  assert.equal(inputIndices.length, 1, "only the value field should be an Input (no editable handle field)");

  const formFieldCount = values.filter((v) => v === FormField).length;
  assert.equal(formFieldCount, 1, "only the value should be wrapped in a FormField");

  // The attributes for the single Input live in the string segment immediately
  // after its marker; assert it is the write-only password value field.
  const inputAttrs = strings[inputIndices[0] + 1] || "";
  assert.ok(
    /type="password"/.test(inputAttrs),
    `the value Input must be a write-only password field, got: ${JSON.stringify(inputAttrs)}`
  );
});
