import { describe, expect, it } from "vitest";

import {
  FORBIDDEN_DECLARATIONS,
  REQUIRED_PROPERTIES,
  REQUIRED_UTILITIES,
  REQUIRED_UTILITY_SELECTORS,
  findContractViolations,
} from "../../scripts/check-token-bundle";

/**
 * Guardrails are code (`.claude/rules/review-discipline.md`): the bundle check
 * only earns its place if it actually fails on the shapes it claims to catch.
 *
 * Each case below reproduces a real regression the emitted-CSS gate exists to
 * stop — most importantly the one that shipped green through every other gate
 * in this repo.
 */

/** A minimal stylesheet that satisfies the whole contract. */
function compliantCss(): string {
  const properties = REQUIRED_PROPERTIES.map((token) => `${token}: 1rem;`).join("\n");
  const utilities = REQUIRED_UTILITIES.map(
    ({ selector, declares }) => `${selector} { ${declares}: 1rem; }`
  ).join("\n");
  return `:root{\n${properties}\n--v2-elevation-1: 0 0 #0000;\n}\n${utilities}`;
}

describe("design-token bundle contract", () => {
  it("passes on a bundle carrying every token and utility", () => {
    expect(findContractViolations(compliantCss())).toEqual({
      missingProperties: [],
      missingUtilities: [],
      forbidden: [],
    });
  });

  // The exact failure this gate was written for: a comment in `app.css`
  // closing early dropped a whole `@theme` block, and every existing gate
  // stayed green because they read source, not output.
  it("fails when a dropped @theme block removes the radius aliases", () => {
    const css = compliantCss()
      .replace(/--radius-control-sm\s*:[^;]*;/, "")
      .replace(/--radius-control-lg\s*:[^;]*;/, "");
    const { missingProperties } = findContractViolations(css);
    expect(missingProperties).toContain("--radius-control-sm");
    expect(missingProperties).toContain("--radius-control-lg");
  });

  // Tree-shaking hits consumer-free tokens first, so these are the canaries
  // for `@theme static` silently regressing to a plain `@theme`.
  it("fails when the consumer-free elevation and spacing steps are tree-shaken", () => {
    const css = compliantCss().replace(/--shadow-e[0-3]\s*:[^;]*;/g, "");
    expect(findContractViolations(css).missingProperties).toEqual(
      expect.arrayContaining(["--shadow-e0", "--shadow-e1", "--shadow-e2", "--shadow-e3"])
    );
  });

  it("fails when a token exists but its utility was never generated", () => {
    const css = compliantCss().replace(".rounded-control-sm { border-radius: 1rem; }", "");
    const { missingProperties, missingUtilities } = findContractViolations(css);
    // The property survives — which is exactly why a source-level check misses this.
    expect(missingProperties).not.toContain("--radius-control-sm");
    expect(missingUtilities).toEqual([".rounded-control-sm"]);
  });

  // `.rounded-control` is a prefix of `.rounded-control-sm`, so substring
  // matching would accept the longer utilities as proof the base one exists.
  // The base is what Button's default size depends on.
  it("does not accept a longer utility as proof the base selector exists", () => {
    const css = compliantCss().replace(".rounded-control { border-radius: 1rem; }", "");
    const { missingUtilities } = findContractViolations(css);
    expect(missingUtilities).toEqual([".rounded-control"]);
  });

  it("accepts a base selector that appears inside a selector group", () => {
    // Tailwind collapses identical declarations into one grouped rule, so a
    // standalone member of a group is a perfectly real utility.
    const css = `${compliantCss()}\n.other, .rounded-control, .third { border-radius: 1rem; }`
      .replace(".rounded-control { border-radius: 1rem; }", "");
    expect(findContractViolations(css).missingUtilities).toEqual([]);
  });

  // Three near-misses that all leave a component using the class unstyled.
  // The gate must reject each: presence of the selector is not the contract,
  // an effective rule is.
  it("rejects an empty base rule", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      ".rounded-control {}"
    );
    expect(findContractViolations(css).missingUtilities).toEqual([".rounded-control"]);
  });

  it("rejects a pseudo-state rule standing in for the base rule", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      ".rounded-control:hover { border-radius: 1rem; }"
    );
    expect(findContractViolations(css).missingUtilities).toEqual([".rounded-control"]);
  });

  it("rejects a descendant rule standing in for the base rule", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      ".rounded-control .child { border-radius: 1rem; }"
    );
    expect(findContractViolations(css).missingUtilities).toEqual([".rounded-control"]);
  });

  it("rejects a base rule that declares something other than the expected property", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      ".rounded-control { color: red; }"
    );
    expect(findContractViolations(css).missingUtilities).toEqual([".rounded-control"]);
  });

  it("rejects an ancestor-scoped rule standing in for the base utility", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      ".wrap .rounded-control { border-radius: 1rem; }"
    );
    expect(findContractViolations(css).missingUtilities).toEqual([".rounded-control"]);
  });

  it("rejects a compound selector standing in for the base utility", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      ".other.rounded-control { border-radius: 1rem; }"
    );
    expect(findContractViolations(css).missingUtilities).toEqual([".rounded-control"]);
  });

  // `content: "border-radius: 1rem"` declares `content`, not `border-radius`.
  // Splitting each declaration at its first top-level `:` is what keeps a
  // quoted value from masquerading as the property.
  it("does not accept the property name inside a quoted value", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      '.rounded-control { content: "border-radius: 1rem"; }'
    );
    expect(findContractViolations(css).missingUtilities).toEqual([".rounded-control"]);
  });

  it("accepts a utility declared inside an at-rule", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      "@media (min-width: 1px) { .rounded-control { border-radius: 1rem; } }"
    );
    expect(findContractViolations(css).missingUtilities).toEqual([]);
  });

  // `content: "; border-radius: 1rem"` is ONE declaration whose property is
  // `content`. Splitting on every `;` would invent a second one that appears
  // to set border-radius.
  it("does not accept a semicolon inside a quoted value as a declaration break", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      '.rounded-control { content: "; border-radius: 1rem" }'
    );
    expect(findContractViolations(css).missingUtilities).toEqual([".rounded-control"]);
  });

  // CSS nesting: the inner rule only applies under `.wrap`, so it is not the
  // standalone utility a component asking for the bare class relies on.
  it("rejects a rule nested inside another style rule", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      ".wrap { .rounded-control { border-radius: 1rem; } }"
    );
    expect(findContractViolations(css).missingUtilities).toEqual([".rounded-control"]);
  });

  // CSS strings honour backslash escapes, so `"x\"; border-radius: 1rem"` is
  // ONE value containing a quote and a semicolon — not a second declaration.
  it("does not accept an escaped quote as the end of a CSS string", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      String.raw`.rounded-control { content: "x\"; border-radius: 1rem"; }`
    );
    expect(findContractViolations(css).missingUtilities).toEqual([".rounded-control"]);
  });

  // A `{` inside a quoted value does not open a block; if the brace walk
  // thought it did, the rest of the sheet would be mis-nested.
  it("does not let a quoted brace alter rule nesting", () => {
    const css = compliantCss().replace(
      ".rounded-control { border-radius: 1rem; }",
      '.rounded-control { content: "{ border-radius: 1rem }"; }'
    );
    expect(findContractViolations(css).missingUtilities).toEqual([".rounded-control"]);
  });

  it("still parses rules that follow a value containing braces and quotes", () => {
    // A pathological but legal value: an unbalanced brace, a semicolon, and an
    // escaped quote. Everything after it must still parse normally.
    const decoy = String.raw`.decoy { content: "{ ; \" }"; }`;
    const css = `${decoy}\n${compliantCss()}`;
    expect(findContractViolations(css).missingUtilities).toEqual([]);
    expect(findContractViolations(css).missingProperties).toEqual([]);
  });

  it("requires the escaped md: variant, not just the base utility", () => {
    const css = compliantCss().replace(".md\\:rounded-control-lg { border-radius: 1rem; }", "");
    expect(findContractViolations(css).missingUtilities).toEqual([".md\\:rounded-control-lg"]);
  });

  // A `var(--token)` reference proves someone asked for the token, not that it
  // resolves. Counting references would make the gate pass on a broken bundle.
  // A regex over raw text also matches `content: "--v2-radius-control:"`.
  // Comparing parsed property names is what makes that impossible.
  it("does not accept a custom property named inside a quoted value", () => {
    const css = compliantCss().replace(
      "--v2-radius-control: 1rem;",
      'content: "--v2-radius-control: 1rem";'
    );
    expect(findContractViolations(css).missingProperties).toContain("--v2-radius-control");
  });

  it("does not accept a var() reference as a declaration", () => {
    const css = compliantCss().replace(
      /--v2-accent-edge\s*:[^;]*;/,
      "border-color: var(--v2-accent-edge);"
    );
    expect(findContractViolations(css).missingProperties).toContain("--v2-accent-edge");
  });

  it("rejects `none` as a flat elevation value", () => {
    const css = compliantCss().replace("--v2-elevation-1: 0 0 #0000;", "--v2-elevation-1: none;");
    expect(findContractViolations(css).forbidden).toHaveLength(1);
    expect(findContractViolations(css).forbidden[0]).toMatch(/never `none`/);
  });

  // The gate's evidence must be emitted CSS, never prose. Tailwind ships a
  // licence banner and Vite can preserve annotations, so comments really do
  // reach `dist/` — a contract satisfiable by a comment would be no contract.
  it("does not accept tokens or utilities that appear only inside comments", () => {
    const properties = REQUIRED_PROPERTIES.map((token) => `${token}: 1rem;`).join("\n");
    const utilities = REQUIRED_UTILITIES.map(
      ({ selector, declares }) => `${selector} { ${declares}: 1rem; }`
    ).join("\n");
    const commentedOut = `/* ${properties}\n${utilities} */\n:root{ --unrelated: 1px; }`;

    const { missingProperties, missingUtilities } = findContractViolations(commentedOut);
    expect(missingProperties).toEqual(REQUIRED_PROPERTIES);
    expect(missingUtilities).toEqual(REQUIRED_UTILITY_SELECTORS);
  });

  it("still sees real declarations that merely sit next to a comment", () => {
    const css = `/* banner: --v2-radius-chip lives here */\n${compliantCss()}`;
    expect(findContractViolations(css).missingProperties).toEqual([]);
  });

  it("does not let a commented-out `none` elevation escape the forbidden check", () => {
    const css = `${compliantCss()}\n/* --v2-elevation-2: none; */`;
    expect(findContractViolations(css).forbidden).toEqual([]);
  });

  it("keeps the forbidden list non-empty so the check cannot silently no-op", () => {
    expect(FORBIDDEN_DECLARATIONS.length).toBeGreaterThan(0);
    expect(REQUIRED_PROPERTIES.length).toBeGreaterThan(0);
    expect(REQUIRED_UTILITIES.length).toBeGreaterThan(0);
    expect(REQUIRED_UTILITIES.every((u) => u.declares.length > 0)).toBe(true);
  });
});
