// Test helper: shared plumbing for rendering one component through the
// `vm.runInNewContext` synthetic-JSX harness (see `src/test/vm-tsx-setup.ts`
// for the `__jsx` factory that produces the `{ strings, values }`-shaped
// tree these helpers walk).
//
// `componentSourceForTest` reads a component's real source, strips its
// `import`/`export` statements (so it can be evaluated standalone inside a
// vm sandbox with its free variables injected as context globals), and
// renames the named export to a plain function the vm script assigns onto
// `globalThis.__testExports`.
//
// `findComponent` / `componentProps` walk the rendered synthetic tree to
// locate a child component reference and read the props passed to it.
//
// Not a test file itself (no `.test.` in the name) so the test runner skips
// it.
import { readFileSync } from "node:fs";

export function componentSourceForTest(fileUrl: URL, exportName: string): string {
  const source = readFileSync(fileUrl, "utf8");
  const lines: string[] = [];
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
    lines.push(
      line.replace(`export function ${exportName}`, `function ${exportName}`),
    );
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ${exportName} };`;
}

export function findComponent(node, component) {
  if (!node || typeof node !== "object") return null;
  if (!Array.isArray(node.values)) return null;
  const componentIndex = node.values.indexOf(component);
  if (componentIndex >= 0) {
    return node;
  }
  for (const value of node.values) {
    const found = findComponent(value, component);
    if (found) return found;
  }
  return null;
}

// HTML attribute names may contain hyphens (for example, data-testid).
export const HTML_ATTRIBUTE_PATTERN = /([A-Za-z][A-Za-z0-9-]*)=\s*$/;

export function componentProps(node, component) {
  const props: Record<string, unknown> = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(HTML_ATTRIBUTE_PATTERN)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}
