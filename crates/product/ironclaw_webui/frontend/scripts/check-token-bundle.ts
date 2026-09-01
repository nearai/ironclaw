#!/usr/bin/env node
/**
 * Assert the design-token contract survives the CSS build.
 *
 * Why this exists as a separate gate: every other check in this repo inspects
 * *source*. The component tests assert Tailwind class STRINGS, the story tests
 * assert rendered structure, and `pnpm build` exits 0 whenever Vite produced a
 * bundle. None of them reads the emitted CSS.
 *
 * That gap is not hypothetical. A malformed comment in `app.css` —
 * `spacing -> p-*&#47;m-*` , where the `*&#47;` closes the comment early — made
 * Tailwind silently drop an entire `@theme` block, all 29 keys. `build`,
 * `test`, `test:storybook` and `build-storybook` all stayed green while the
 * emitted CSS carried no `rounded-control` utility at all, which would have
 * shipped every button with square corners.
 *
 * So: read what actually reached `dist/`, and fail loudly when a token or a
 * utility the design system promises is not there.
 */

import { readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Custom properties that must be declared in the emitted CSS.
 *
 * Tailwind tree-shakes `@theme` keys that no generated utility references,
 * which is why the block in `app.css` is `@theme static`. These are the keys
 * that would vanish first if that keyword were dropped — they have no consumer
 * yet and exist as a contract for Phase 3a to build on.
 */
export const REQUIRED_PROPERTIES = [
  // Radius scale — the axis Button consumes today.
  "--v2-radius-chip",
  "--v2-radius-field",
  "--v2-radius-control-sm",
  "--v2-radius-control",
  "--v2-radius-control-lg",
  "--v2-radius-control-xl",
  "--v2-radius-surface",
  "--v2-radius-surface-lg",
  "--v2-radius-pill",
  // Tailwind-namespace aliases: these are what actually generate utilities.
  "--radius-control",
  "--radius-control-sm",
  "--radius-control-lg",
  "--radius-control-xl",
  // Elevation. `--shadow-e*` has no consumer yet and is the canary for
  // `@theme static` regressing to a plain `@theme`.
  "--shadow-e0",
  "--shadow-e1",
  "--shadow-e2",
  "--shadow-e3",
  // Spacing and motion — likewise consumer-free today.
  "--spacing-gutter",
  "--spacing-inset-sm",
  "--spacing-inset",
  "--spacing-inset-lg",
  "--spacing-stack-sm",
  "--spacing-stack",
  "--spacing-stack-lg",
  "--ease-standard",
  "--ease-emphasized",
  "--ease-exit",
  // Motion vocabulary consumed as `var()` directly (no Tailwind namespace).
  "--v2-duration-instant",
  "--v2-duration-fast",
  "--v2-duration-base",
  "--v2-duration-slow",
  // Editorial type steps.
  "--text-title-sm",
  "--text-title",
  "--text-title-lg",
  "--text-display",
  "--text-ui-xs",
  // Button's accent surface.
  "--v2-accent-gradient",
  "--v2-accent-gradient-hover",
  "--v2-accent-edge",
  "--v2-accent-glow",
];

/**
 * Utility class selectors that must be generated. A declared token with no
 * emitted utility is exactly the failure the comment bug produced: the custom
 * property was fine, but `.rounded-control` never existed, so the class on
 * `<Button>` matched nothing.
 *
 * Written as they appear in the bundle — Tailwind escapes the `:` of a variant.
 */
export const REQUIRED_UTILITIES = [
  { selector: ".rounded-control", declares: "border-radius" },
  { selector: ".rounded-control-sm", declares: "border-radius" },
  { selector: ".rounded-control-xl", declares: "border-radius" },
  { selector: ".md\\:rounded-control-lg", declares: "border-radius" },
];

/** Just the selectors — used for reporting and by the contract tests. */
export const REQUIRED_UTILITY_SELECTORS = REQUIRED_UTILITIES.map(({ selector }) => selector);

/**
 * Declarations that must NOT appear — see `app.css` on why flat is `0 0 #0000`.
 *
 * Matched against PARSED declarations, so a quoted value mentioning `none`
 * cannot trigger a false failure any more than it could satisfy a contract.
 */
export const FORBIDDEN_DECLARATIONS = [
  {
    matches: ({ property, value }: Declaration) =>
      /^--v2-elevation-\d$/.test(property) && value.trim() === "none",
    reason:
      "flat elevation must be `0 0 #0000`, never `none` — Tailwind composes box-shadow as a comma list where `none` is only legal alone, so `shadow-e1 ring-2` would silently lose its ring",
  },
];

/**
 * Strip `/* … *\/` spans so a comment can never stand in as evidence.
 *
 * Tailwind emits a licence banner and Vite can preserve annotations, so
 * comments genuinely do reach `dist/`. A comment mentioning `--v2-radius-chip:`
 * or `.rounded-control` would otherwise satisfy this contract while no rule was
 * emitted at all — which is the exact shape of failure the gate exists to
 * catch, so it must not be satisfiable by prose.
 */
export function stripCssComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, " ");
}

export interface Declaration {
  property: string;
  value: string;
}

export interface StyleRule {
  /** The selector list, split into its members and trimmed. */
  selectors: string[];
  /** Declarations made DIRECTLY in this rule's own block. */
  declarations: Declaration[];
  /**
   * True when this rule is nested inside another STYLE rule (CSS nesting), so
   * its selector is scoped by a parent. `@media`/`@supports` nesting does not
   * set this: a conditional group does not narrow the selector.
   */
  nestedUnderSelector: boolean;
}

/**
 * Walk CSS text, reporting only the characters that are STRUCTURE.
 *
 * A structural scanner that ignores strings and escapes is not parsing CSS, it
 * is guessing at it: `content: "x\\"; border-radius: 1rem"` is one declaration
 * whose value happens to contain a quote and a semicolon, and `content: "{"`
 * does not open a block. Every scanner below shares this walk so none of them
 * can drift from the others.
 */
function* scanStructure(text: string): Generator<{ char: string; index: number }> {
  let quote: string | null = null;
  let escaped = false;
  for (let index = 0; index < text.length; index++) {
    const char = text[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (char === "\\") {
      escaped = true;
      continue;
    }
    if (quote) {
      if (char === quote) quote = null;
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }
    yield { char, index };
  }
}

/** Split on top-level commas only — `:is(a, b)` is one selector, not two. */
function splitSelectorList(prelude: string): string[] {
  const cuts: number[] = [];
  let depth = 0;
  for (const { char, index } of scanStructure(prelude)) {
    if (char === "(" || char === "[") depth++;
    else if (char === ")" || char === "]") depth--;
    else if (char === "," && depth === 0) cuts.push(index);
  }
  const members: string[] = [];
  let from = 0;
  for (const cut of [...cuts, prelude.length]) {
    const member = prelude.slice(from, cut).trim();
    if (member) members.push(member);
    from = cut + 1;
  }
  return members;
}

/**
 * Split a block into declaration chunks on structural semicolons.
 *
 * A naive `split(";")` lets `content: "; border-radius: 1rem"` masquerade as
 * two declarations, the second of which appears to set `border-radius`.
 */
function splitDeclarations(flat: string): string[] {
  const cuts: number[] = [];
  let depth = 0;
  for (const { char, index } of scanStructure(flat)) {
    if (char === "(") depth++;
    else if (char === ")") depth--;
    else if (char === ";" && depth === 0) cuts.push(index);
  }
  const chunks: string[] = [];
  let from = 0;
  for (const cut of [...cuts, flat.length]) {
    chunks.push(flat.slice(from, cut));
    from = cut + 1;
  }
  return chunks;
}

/**
 * Declarations made directly in a block.
 *
 * Nested blocks are removed first, and each declaration is split at its FIRST
 * structural `:` — so `content: "border-radius: 1rem"` yields the property
 * `content`. Quoted text is never a declaration.
 */
function parseDeclarations(body: string): Declaration[] {
  // Strip nested blocks using structural braces only, so a `{` inside a string
  // cannot swallow the rest of the block.
  let flat = "";
  let depth = 0;
  let cursor = 0;
  for (const { char, index } of scanStructure(body)) {
    if (char !== "{" && char !== "}") continue;
    if (char === "{") {
      if (depth === 0) flat += body.slice(cursor, index);
      depth++;
    } else {
      depth--;
      if (depth === 0) cursor = index + 1;
    }
  }
  if (depth === 0) flat += body.slice(cursor);

  const declarations: Declaration[] = [];
  for (const chunk of splitDeclarations(flat)) {
    let depthInner = 0;
    let split = -1;
    for (const { char, index } of scanStructure(chunk)) {
      if (char === "(") depthInner++;
      else if (char === ")") depthInner--;
      else if (char === ":" && depthInner === 0) {
        split = index;
        break;
      }
    }
    if (split === -1) continue;
    const property = chunk.slice(0, split).trim();
    if (property) declarations.push({ property, value: chunk.slice(split + 1).trim() });
  }
  return declarations;
}

/**
 * Every style rule in the sheet, descending through at-rules (`@media`,
 * `@supports`, `@layer`) and CSS nesting.
 *
 * Parsing rather than pattern-matching is the point. A regex over raw text
 * cannot tell `.rounded-control{…}` from `.wrap .rounded-control{…}`,
 * `.other.rounded-control{…}` or `.wrap { .rounded-control{…} }` without
 * re-implementing selector grammar badly — and each of those styles something
 * other than the bare utility a component asking for the class relies on.
 */
export function parseStyleRules(css: string): StyleRule[] {
  const rules: StyleRule[] = [];
  const walk = (text: string, nestedUnderSelector: boolean): void => {
    // Structural braces only: a `{` inside a quoted value is not a block.
    const braces = [...scanStructure(text)].filter(
      ({ char }) => char === "{" || char === "}"
    );
    let preludeFrom = 0;
    let position = 0;
    while (position < braces.length) {
      const open = braces[position];
      if (open.char !== "{") {
        // A stray `}` closes whatever we were reading; start a fresh prelude.
        preludeFrom = open.index + 1;
        position++;
        continue;
      }
      let depth = 1;
      let cursor = position + 1;
      while (cursor < braces.length && depth > 0) {
        depth += braces[cursor].char === "{" ? 1 : -1;
        cursor++;
      }
      const closeIndex = depth === 0 ? braces[cursor - 1].index : text.length;
      const body = text.slice(open.index + 1, closeIndex);
      const prelude = text.slice(preludeFrom, open.index);
      // `;` ends a statement at-rule (`@import …;`) — only the tail is a prelude.
      const lastStatement = [...scanStructure(prelude)]
        .filter(({ char }) => char === ";")
        .pop();
      const head = prelude.slice(lastStatement ? lastStatement.index + 1 : 0).trim();
      if (head.startsWith("@")) {
        // A conditional group rule does not narrow the selector, so its
        // children keep whatever nesting context we already had.
        walk(body, nestedUnderSelector);
      } else if (head) {
        rules.push({
          selectors: splitSelectorList(head),
          declarations: parseDeclarations(body),
          nestedUnderSelector,
        });
        // Anything inside a style rule IS scoped by that rule's selector.
        walk(body, true);
      }
      preludeFrom = closeIndex + 1;
      position = cursor;
    }
  };
  walk(css, false);
  return rules;
}

export interface ContractViolations {
  missingProperties: string[];
  missingUtilities: string[];
  forbidden: string[];
}

/** @param rawCss concatenated text of every emitted stylesheet */
export function findContractViolations(rawCss: string): ContractViolations {
  const rules = parseStyleRules(stripCssComments(rawCss));
  const allDeclarations = rules.flatMap((rule) => rule.declarations);

  // Compare PARSED property names. A regex over raw text also matches
  // `content: "--v2-radius-control:"`, letting a quoted value satisfy the
  // contract — and a `var(--token)` reference proves only that something asked
  // for the token, not that it is declared anywhere.
  const declaredProperties = new Set(allDeclarations.map(({ property }) => property));
  const missingProperties = REQUIRED_PROPERTIES.filter(
    (token) => !declaredProperties.has(token)
  );

  // A utility counts only when some STANDALONE rule lists it as an exact
  // selector-list member and that rule's own block declares the property the
  // utility exists to set. Excluded by construction: `.wrap .rounded-control`,
  // `.other.rounded-control`, `.rounded-control:hover`, `.rounded-control-sm`,
  // `.wrap { .rounded-control { … } }`, and an empty block.
  const missingUtilities = REQUIRED_UTILITIES.filter(
    ({ selector, declares }) =>
      !rules.some(
        (rule) =>
          !rule.nestedUnderSelector &&
          rule.selectors.includes(selector) &&
          rule.declarations.some(({ property }) => property === declares)
      )
  ).map(({ selector }) => selector);

  const forbidden = FORBIDDEN_DECLARATIONS.filter(({ matches }) =>
    allDeclarations.some(matches)
  ).map(({ reason }) => reason);

  return { missingProperties, missingUtilities, forbidden };
}

export function readBundledCss(assetsDir: string): string {
  const files = readdirSync(assetsDir).filter((name) => name.endsWith(".css"));
  if (files.length === 0) {
    throw new Error(`No .css emitted in ${assetsDir} — run \`pnpm build\` first.`);
  }
  return files.map((name) => readFileSync(join(assetsDir, name), "utf8")).join("\n");
}

function main(): void {
  const assetsDir = process.argv[2] ?? "dist/assets";
  const css = readBundledCss(assetsDir);
  const { missingProperties, missingUtilities, forbidden } = findContractViolations(css);

  if (missingProperties.length === 0 && missingUtilities.length === 0 && forbidden.length === 0) {
    console.log(
      `Design-token bundle contract OK — ${REQUIRED_PROPERTIES.length} properties and ` +
        `${REQUIRED_UTILITIES.length} utilities present in ${assetsDir}.`
    );
    return;
  }

  console.error("Design-token contract broken in the EMITTED CSS.\n");
  if (missingProperties.length > 0) {
    console.error("Missing custom properties:");
    for (const token of missingProperties) console.error(`  ${token}`);
    console.error(
      "\n  A whole block vanishing usually means `app.css` lost `@theme static`,\n" +
        "  or a comment above it closed early (a literal `*/` inside the comment\n" +
        "  body — `p-*/m-*` is the known trap).\n"
    );
  }
  if (missingUtilities.length > 0) {
    console.error("Missing utility selectors:");
    for (const selector of missingUtilities) console.error(`  ${selector}`);
    console.error(
      "\n  The token may exist while its utility does not — or the rule exists\n" +
        "  but is empty, or only a `:hover`/descendant variant survived. A\n" +
        "  component using the class then renders with no radius at all.\n"
    );
  }
  if (forbidden.length > 0) {
    console.error("Forbidden declarations:");
    for (const reason of forbidden) console.error(`  ${reason}`);
  }
  process.exit(1);
}

// Only run when invoked as a CLI, so the unit test can import the helpers.
// Same shape as check-bundle-budgets.ts next door.
const invokedPath = process.argv[1] ? resolve(process.argv[1]) : undefined;
if (invokedPath === fileURLToPath(import.meta.url)) main();
