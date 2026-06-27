import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function sourceForTest(path, exportNames) {
  const source = readFileSync(new URL(path, import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ${exportNames.join(", ")} };`;
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
      if (typeof value === "string") scalars.push(value);
    }
  });
  return scalars;
}

function visibleLanguageCodes(root) {
  return Array.from(
    new Set(collectScalars(root).filter((value) => ["en", "de", "ja"].includes(value))),
  );
}

function deepValuesAfter(root, fragment) {
  const values = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !Array.isArray(node.values)) return;
    node.strings.forEach((part, index) => {
      if (part.includes(fragment)) values.push(node.values[index]);
    });
  });
  return values;
}

function findComponentNodes(root, component) {
  const nodes = [];
  visit(root, (node) => {
    if (Array.isArray(node.values) && node.values.includes(component)) nodes.push(node);
  });
  return nodes;
}

function componentProps(node, component) {
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function matchesSearch(query, values) {
  const normalized = String(query || "").trim().toLowerCase();
  if (!normalized) return true;
  return values.some((value) => String(value || "").toLowerCase().includes(normalized));
}

function renderLanguageTab({ lang = "en", searchQuery = "" } = {}) {
  const calls = [];
  const SettingsSearchEmpty = "SettingsSearchEmpty";
  const context = {
    AVAILABLE_LANGUAGES: [
      { code: "en", name: "English", native: "English" },
      { code: "de", name: "German", native: "Deutsch" },
      { code: "ja", name: "Japanese", native: "Japanese" },
    ],
    Card: "Card",
    SettingsSearchEmpty,
    globalThis: {},
    html,
    matchesSearch,
    useI18n: () => ({
      lang,
      setLang: (next) => calls.push(next),
    }),
    useT: () => (key, params = {}) => {
      if (key === "settings.noMatchingSettings") return `No matching settings for ${params.query}`;
      return key;
    },
  };

  vm.runInNewContext(
    sourceForTest("./language-tab.js", ["LanguageTab"]),
    context,
  );
  const rendered = context.globalThis.__testExports.LanguageTab({ searchQuery });
  return { rendered, calls, SettingsSearchEmpty };
}

test("LanguageTab shows current language and all languages by default", () => {
  const { rendered } = renderLanguageTab({ lang: "de" });
  const text = collectScalars(rendered);

  assert.ok(text.includes("lang.title"));
  assert.ok(text.includes("lang.description"));
  assert.ok(text.includes("lang.current"));
  assert.ok(text.includes("Deutsch"));
  assert.ok(text.includes("German"));
  assert.deepEqual(visibleLanguageCodes(rendered), ["en", "de", "ja"]);
});

test("LanguageTab filters by code, English name, and native label", () => {
  assert.deepEqual(visibleLanguageCodes(renderLanguageTab({ searchQuery: "de" }).rendered), ["de"]);
  assert.deepEqual(visibleLanguageCodes(renderLanguageTab({ searchQuery: "german" }).rendered), ["de"]);
  assert.deepEqual(visibleLanguageCodes(renderLanguageTab({ searchQuery: "Deutsch" }).rendered), ["de"]);
});

test("LanguageTab language buttons call setLang with the selected code", () => {
  const { rendered, calls } = renderLanguageTab();
  const handlers = deepValuesAfter(rendered, "onClick=");

  assert.equal(handlers.length, 3);
  handlers[1]();

  assert.deepEqual(calls, ["de"]);
});

test("LanguageTab renders the shared empty search state when filtered out", () => {
  const { rendered, SettingsSearchEmpty } = renderLanguageTab({ searchQuery: "missing" });
  const emptyNodes = findComponentNodes(rendered, SettingsSearchEmpty);

  assert.equal(emptyNodes.length, 1);
  assert.equal(componentProps(emptyNodes[0], SettingsSearchEmpty).query, "missing");
});
