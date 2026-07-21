#!/usr/bin/env node
/**
 * check-design-tokens.mjs — design-token ratchet for the WebUI v2.
 *
 * Scans crates/ironclaw_webui/frontend/src/** (excluding vendor/,
 * dist/, node_modules/, and colocated *.test.* files) for:
 *
 * 1. Hardcoded color literals — hex (#abc / #aabbcc / #aabbccdd) and
 *    rgb()/rgba() calls. Colors must come from the --v2-* custom
 *    properties defined in frontend/src/styles/app.css.
 * 2. Legacy alias utility classes (text-white, bg-white/*, iron-* /
 *    signal / copper / mint palette classes, red-* status classes).
 *    These are compat shims remapped by app.css to theme tokens —
 *    several contradict their literal meaning (`.text-white` renders
 *    the theme ink color, i.e. dark in light mode), so new code must
 *    use the semantic token classes instead. See DESIGN_SYSTEM.md
 *    ("Provenance & reconciliation" + the color rules in §2).
 *
 * Pre-existing occurrences are grandfathered in
 * scripts/design-tokens-baseline.json. The check fails when any file
 * has MORE raw colors than its baseline entry (new files start at 0),
 * so the count can only ratchet down.
 *
 * Usage:
 *   node scripts/check-design-tokens.mjs                 # check (CI/agent hook)
 *   node scripts/check-design-tokens.mjs --update-baseline
 *     # rewrite the baseline to current counts — only run this in a
 *     # PR that *removes* violations, so the ratchet goes down.
 */
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SCAN_ROOTS = [
  join(REPO_ROOT, "crates/ironclaw_webui/frontend/src"),
  join(REPO_ROOT, "crates/ironclaw_webui/frontend/packages/design-system/src"),
];
const BASELINE_PATH = join(REPO_ROOT, "scripts/design-tokens-baseline.json");

// color-mix(...) over --v2-* vars is sanctioned; bare rgb()/rgba()
// and hex literals are not. Hex requires a word boundary after so a
// longer id string does not double-count.
const HEX_RE = /#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})\b/g;
// (?<![\w-]) instead of \b so `rgba(` inside Tailwind arbitrary
// values (`shadow-[0_24px_rgba(...)]` — underscores are word chars)
// is still caught.
const RGB_RE = /(?<![A-Za-z0-9-])rgba?\(/g;
// Legacy alias utilities (remapped by the app.css compat shim /
// index.html @theme block). Matches the utility with any prefix
// variant (hover:, focus:, md:, …) via the leading quote/space/colon
// boundary. `text-white` is the canonical trap: it renders
// --v2-text-strong (dark ink in light mode), not white.
const LEGACY_ALIAS_RE =
  /(?<![A-Za-z0-9_/[-])(?:text-white|(?:text|bg|border)-(?:iron-\d+|signal|copper|mint|red-\d+)|(?:bg|border)-white\/)/g;

const SOURCE_EXTENSIONS = [".js", ".mjs", ".jsx", ".ts", ".tsx"];
const TEST_MARKERS = [".test.", ".spec."];

function collectJsFiles(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      if (entry === "vendor" || entry === "dist" || entry === "node_modules") continue;
      collectJsFiles(path, out);
    } else if (
      SOURCE_EXTENSIONS.some((ext) => path.endsWith(ext)) &&
      !TEST_MARKERS.some((marker) => entry.includes(marker)) &&
      !path.endsWith(".d.ts")
    ) {
      out.push(path);
    }
  }
  return out;
}

function findViolations(source) {
  const violations = [];
  // Blank out block comments first (issue refs like `/* see #123 */`
  // are valid hex-ish strings), replacing non-newline chars with
  // spaces so line numbers in the report stay accurate.
  const withoutBlockComments = source.replace(/\/\*[\s\S]*?\*\//g, (m) =>
    m.replace(/[^\n]/g, " ")
  );
  const lines = withoutBlockComments.split("\n");
  lines.forEach((rawLine, index) => {
    // Drop line comments (whitespace-preceded `//` so `https://` in
    // string literals survives) — a comment *mentioning* text-white
    // or a hex value is not a violation.
    const line = rawLine.replace(/(^|\s)\/\/.*$/, "$1");
    for (const re of [HEX_RE, RGB_RE, LEGACY_ALIAS_RE]) {
      re.lastIndex = 0;
      let match;
      while ((match = re.exec(line)) !== null) {
        violations.push({ line: index + 1, text: match[0] });
      }
    }
  });
  return violations;
}

function loadBaseline() {
  try {
    return JSON.parse(readFileSync(BASELINE_PATH, "utf8"));
  } catch {
    return {};
  }
}

const updateBaseline = process.argv.includes("--update-baseline");
const counts = {};
const details = {};

for (const file of SCAN_ROOTS.flatMap((root) => collectJsFiles(root)).sort()) {
  const rel = relative(REPO_ROOT, file);
  const violations = findViolations(readFileSync(file, "utf8"));
  if (violations.length > 0) {
    counts[rel] = violations.length;
    details[rel] = violations;
  }
}

if (updateBaseline) {
  writeFileSync(BASELINE_PATH, `${JSON.stringify(counts, null, 2)}\n`);
  console.log(
    `design-tokens baseline updated: ${Object.keys(counts).length} file(s), ` +
      `${Object.values(counts).reduce((a, b) => a + b, 0)} grandfathered literal(s).`
  );
  process.exit(0);
}

const baseline = loadBaseline();
let failed = false;

for (const [rel, count] of Object.entries(counts)) {
  const allowed = baseline[rel] ?? 0;
  if (count > allowed) {
    failed = true;
    console.error(
      `\n${rel}: ${count} raw color literal(s), baseline allows ${allowed}.`
    );
    for (const violation of details[rel]) {
      console.error(`  line ${violation.line}: ${violation.text}`);
    }
  }
}

if (failed) {
  console.error(
    "\nHardcoded colors and legacy alias utilities (text-white, iron-*/" +
      "signal/copper/mint/red-* classes, bg-white/*) are not allowed in " +
      "new WebUI v2 JS — use the --v2-* design tokens (var(--v2-...)) " +
      "instead. In particular `text-white` does NOT render white: the " +
      "compat shim remaps it to the theme ink color; use " +
      "text-[var(--v2-on-accent)] on accent fills. See " +
      "crates/ironclaw_webui/DESIGN_SYSTEM.md. If you removed other " +
      "violations in the same file, re-run with --update-baseline."
  );
  process.exit(1);
}

console.log(
  "design-tokens check passed: no new raw color literals or legacy alias utilities."
);
