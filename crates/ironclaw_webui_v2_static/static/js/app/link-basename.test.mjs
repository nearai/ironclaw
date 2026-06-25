import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import test from "node:test";
import { ROUTER_BASENAME } from "./router-href.js";

// Guards the whole webui_v2 router-navigation surface against the doubled-
// basename bug (PR #5235 shipped a chat `<Link to="/v2/logs">` that react-router
// resolved to "/v2/v2/logs"). react-router prepends ROUTER_BASENAME to every
// `<Link to>` / `<Navigate to>` / `navigate()` target, so any such target that
// is itself a "/v2"-prefixed string literal resolves to a broken doubled path.
// Only raw `<a href>` navigations (which bypass the router) may carry "/v2".

const SRC_ROOT = fileURLToPath(new URL("..", import.meta.url));

function jsSourceFiles(dir) {
  const files = [];
  for (const entry of readdirSync(dir)) {
    const full = path.join(dir, entry);
    if (statSync(full).isDirectory()) {
      if (entry === "vendor" || entry === "dist" || entry === "node_modules") {
        continue;
      }
      files.push(...jsSourceFiles(full));
      continue;
    }
    if (!entry.endsWith(".js")) continue;
    if (entry.endsWith(".test.js")) continue;
    files.push(full);
  }
  return files;
}

// Matches string-literal router-navigation targets: `to="..."` props (Link /
// NavLink / Navigate) and `navigate("...")` calls. The `to` lookbehind rejects
// hyphenated attributes (e.g. `data-to=`) so only the bare router prop matches.
// Dynamic targets (`to=${...}`, `navigate(expr)`) carry no literal and are
// covered by per-builder tests such as logs-data's "basename-relative" guard.
const TARGET_PATTERNS = [
  /(?<![\w-])to=("|')([^"'`]*)\1/g,
  /\bnavigate\(\s*("|')([^"'`]*)\1/g,
];

test("router navigation targets are basename-relative (no doubled /v2)", () => {
  const offenders = [];
  let literalTargets = 0;

  for (const file of jsSourceFiles(SRC_ROOT)) {
    const source = readFileSync(file, "utf8");
    for (const pattern of TARGET_PATTERNS) {
      for (const match of source.matchAll(pattern)) {
        const target = match[2];
        if (!target.startsWith("/")) continue; // skip relative / hash targets
        literalTargets += 1;
        if (
          target === ROUTER_BASENAME ||
          target.startsWith(`${ROUTER_BASENAME}/`)
        ) {
          offenders.push(`${path.relative(SRC_ROOT, file)}: ${target}`);
        }
      }
    }
  }

  // Sanity: the scan actually found router targets (guards against a regex that
  // silently matches nothing and passes vacuously).
  assert.ok(literalTargets > 0, "expected to scan some router navigation targets");
  assert.deepEqual(
    offenders,
    [],
    `router targets must be basename-relative (drop the "${ROUTER_BASENAME}" prefix):\n${offenders.join("\n")}`,
  );
});
