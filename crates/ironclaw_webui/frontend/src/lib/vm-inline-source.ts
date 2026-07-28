// Test helper: return a module's source with its `import`/`export` statements
// stripped, so it can be prepended into a `vm.runInNewContext` script alongside
// another module that imports from it.
//
// The hook test harnesses (`useChat-send.test.ts`, `useExtensions-*.test.ts`)
// load a hook's source, strip its imports, and run it in a fresh vm context with
// dependencies injected as globals. When a hook imports helpers that must be
// *compiled inside* the vm — window-dependent primitives (e.g. `openAuthPopup`)
// that resolve the per-test `window`, or a sibling hook the test drives through
// the caller — inject those by inlining their (import/export-stripped) source
// rather than injecting a closure from Node's realm.
//
// Not a test file itself (no `.test.` in the name) so the test runner skips it.
import { readFileSync } from "node:fs";

export function moduleSourceForVm(fileUrl) {
  const source = readFileSync(fileUrl, "utf8");
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
    lines.push(line.replace(/^export /, ""));
  }
  return lines.join("\n");
}
