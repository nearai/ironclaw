// Test helper: return the source of `product-auth-oauth-events.ts` with its
// `import`/`export` statements stripped, so it can be prepended into a
// `vm.runInNewContext` script alongside a hook that imports from it.
//
// The window-dependent primitives (e.g. `openAuthPopup`) must be *compiled
// inside* the vm so they resolve the per-test `window`, which an injected
// closure from Node's realm cannot. Inline the shared source instead. See
// `vm-inline-source.ts` for the generic stripper.
//
// Not a test file itself (no `.test.` in the name) so the test runner skips it.
import { moduleSourceForVm } from "./vm-inline-source";

export function productAuthOAuthEventsSource() {
  return moduleSourceForVm(
    new URL("./product-auth-oauth-events.ts", import.meta.url),
  );
}
