#!/usr/bin/env node
/**
 * check-design-tokens.mjs — design-token ratchet for the WebUI v2.
 *
 * Scans crates/ironclaw_webui_v2/static/js/** (excluding vendor/,
 * dist/, and colocated *.test.{js,mjs} files) for hardcoded color
 * literals — hex (#abc / #aabbcc / #aabbccdd) and rgb()/rgba() calls.
 * The design system requires colors to come from the --v2-* custom
 * properties defined in static/styles/app.css (see
 * crates/ironclaw_webui_v2/DESIGN_SYSTEM.md), so raw color literals
 * in JS are violations.
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
const SCAN_ROOT = join(REPO_ROOT, "crates/ironclaw_webui_v2/static/js");
const BASELINE_PATH = join(REPO_ROOT, "scripts/design-tokens-baseline.json");

// color-mix(...) over --v2-* vars is sanctioned; bare rgb()/rgba()
// and hex literals are not. Hex requires a word boundary after so a
// longer id string does not double-count.
const HEX_RE = /#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})\b/g;
// (?<![\w-]) instead of \b so `rgba(` inside Tailwind arbitrary
// values (`shadow-[0_24px_rgba(...)]` — underscores are word chars)
// is still caught.
const RGB_RE = /(?<![A-Za-z0-9-])rgba?\(/g;

function collectJsFiles(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      if (entry === "vendor" || entry === "dist") continue;
      collectJsFiles(path, out);
    } else if (
      path.endsWith(".js") &&
      !path.endsWith(".test.js") &&
      !path.endsWith(".test.mjs")
    ) {
      out.push(path);
    }
  }
  return out;
}

function findViolations(source) {
  const violations = [];
  const lines = source.split("\n");
  lines.forEach((line, index) => {
    for (const re of [HEX_RE, RGB_RE]) {
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

for (const file of collectJsFiles(SCAN_ROOT).sort()) {
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
    "\nHardcoded colors are not allowed in WebUI v2 JS — use the --v2-* " +
      "design tokens (var(--v2-...)) instead. See " +
      "crates/ironclaw_webui_v2/DESIGN_SYSTEM.md. If you removed other " +
      "violations in the same file, re-run with --update-baseline."
  );
  process.exit(1);
}

console.log("design-tokens check passed: no new raw color literals.");
