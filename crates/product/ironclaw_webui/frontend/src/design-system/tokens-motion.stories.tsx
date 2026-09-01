import type { Meta, StoryObj } from "@storybook/react-vite";
import { useEffect, useState } from "react";
import { expect, waitFor } from "storybook/test";

import { Spinner } from "./spinner";

/**
 * The motion vocabulary from `src/styles/app.css`.
 *
 * Each row prints the value READ from the live custom property rather than a
 * literal copied into this file: Phase 3a (#7781 WS3) retunes the durations in
 * `app.css` alone, and a duplicated `250ms` here would silently disagree with
 * the token it documents.
 */

const DURATIONS = [
  { token: "--v2-duration-instant", note: "hover/press feedback" },
  { token: "--v2-duration-fast", note: "state changes on a control" },
  { token: "--v2-duration-base", note: "a surface entering or leaving" },
  { token: "--v2-duration-slow", note: "a full-screen or layout transition" },
];

const EASES = [
  { token: "--v2-ease-standard", note: "M3 standard — state changes" },
  { token: "--v2-ease-emphasized", note: "M3 emphasized — entering, expanding" },
  { token: "--v2-ease-exit", note: "M3 accelerate — dismissal" },
];

const ALL_TOKENS = [...DURATIONS, ...EASES].map(({ token }) => token);
const DURATION_TOKENS = DURATIONS.map(({ token }) => token);

function useResolved(token: string): string {
  const [resolved, setResolved] = useState("");
  useEffect(() => {
    setResolved(getComputedStyle(document.documentElement).getPropertyValue(token).trim());
  }, [token]);
  return resolved;
}

function Row({ token, note }: { token: string; note: string }) {
  const resolved = useResolved(token);
  return (
    <div className="flex items-baseline justify-between gap-4 rounded-[12px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-3">
      <span className="font-mono text-ui-sm text-[var(--v2-text-strong)]">{token}</span>
      <span className="flex min-w-0 items-baseline gap-3">
        <span className="truncate text-[0.6875rem] text-[var(--v2-text-muted)]">{note}</span>
        <span
          data-testid={`value-${token}`}
          className="shrink-0 font-mono text-[0.625rem] text-[var(--v2-text-faint)]"
        >
          {resolved || "—"}
        </span>
      </span>
    </div>
  );
}

/**
 * Walk the parsed CSSOM for the `prefers-reduced-motion: reduce` block and
 * return the duration tokens it collapses to `0ms` **on a rule that actually
 * targets the element the tokens are read from**.
 *
 * The selector match is the load-bearing part. Counting `0ms` declarations
 * alone would pass on a rule scoped to something that never matches — the
 * media query would be active for a reduced-motion user while `:root` kept its
 * normal durations, which is precisely the regression this is meant to stop.
 *
 * Reading the CSSOM rather than emulating the media query keeps the check
 * inside the story, so it runs in the static catalog build as well as the
 * Chromium suite. The one thing it does not prove is the resolved cascade
 * under emulation; everything it does assert — a renamed token, a dropped
 * declaration, a selector that stopped matching `:root`, a changed media
 * condition — is a real regression it fails on.
 */
function reducedMotionZeroedTokens(): string[] {
  const target = document.documentElement;
  const zeroed = new Set<string>();
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRule[];
    try {
      rules = Array.from(sheet.cssRules);
    } catch {
      continue; // cross-origin sheet — not ours
    }
    for (const rule of rules) {
      if (!(rule instanceof CSSMediaRule)) continue;
      // `reduce` specifically: a `no-preference` block would zero nothing.
      if (!/prefers-reduced-motion\s*:\s*reduce/.test(rule.conditionText)) continue;
      for (const inner of Array.from(rule.cssRules)) {
        if (!(inner instanceof CSSStyleRule)) continue;
        // The rule must apply to the element the tokens are read from.
        if (!target.matches(inner.selectorText)) continue;
        for (const token of DURATION_TOKENS) {
          if (inner.style.getPropertyValue(token).trim() === "0ms") zeroed.add(token);
        }
      }
    }
  }
  return [...zeroed];
}

const meta = {
  title: "Tokens/Motion",
} satisfies Meta;

export default meta;
type Story = StoryObj;

export const Motion: Story = {
  render: () => (
    <div className="flex max-w-xl flex-col gap-6 text-[var(--v2-text)]">
      <div className="flex items-center gap-4">
        <Spinner className="h-8 w-8 text-[var(--v2-accent-text)]" />
        <div>
          <div className="font-mono text-ui-sm text-[var(--v2-text-strong)]">.v2-spin</div>
          <div className="text-ui-sm text-[var(--v2-text-muted)]">
            0.8s linear infinite — the one always-on animation
          </div>
        </div>
      </div>
      <p className="text-ui leading-6 text-[var(--v2-text-muted)]">
        The app ships a static-motion policy: a global{" "}
        <code className="rounded bg-[var(--v2-surface-soft)] px-1 font-mono text-ui-sm">
          {"* { animation: none !important }"}
        </code>{" "}
        rule disables transitions and animations by default. Only a few class-scoped exceptions
        (notably <code className="font-mono text-ui-sm">.v2-spin</code> for loading spinners) opt back
        in, and <code className="font-mono text-ui-sm">prefers-reduced-motion</code> re-suppresses even
        those.
      </p>

      <section>
        <h3 className="mb-1 font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]">
          Motion vocabulary
        </h3>
        <p className="mb-3 text-ui leading-6 text-[var(--v2-text-muted)]">
          The tokens below are the vocabulary Phase 4 (#7782 WS4) opts components back into — they
          do not lift the policy above. They exist so an animation says how long and with what curve
          by NAME: under <code className="font-mono text-ui-sm">prefers-reduced-motion</code> every
          duration token collapses to <code className="font-mono text-ui-sm">0ms</code> at the token
          layer, so a component that takes its duration from one is gated by construction rather
          than by a reviewer noticing.
        </p>
        <div className="flex flex-col gap-2">
          {DURATIONS.map(({ token, note }) => (
            <Row key={token} token={token} note={note} />
          ))}
          {EASES.map(({ token, note }) => (
            <Row key={token} token={token} note={note} />
          ))}
        </div>
      </section>
    </div>
  ),
  // Two assertions, because a render proves neither. A dropped or renamed
  // token yields an empty string and the page still looks fine; and the
  // reduced-motion collapse — the property #7782 WS4 is told it inherits for
  // free — has no visible surface at all.
  play: async ({ canvas }) => {
    const cells = await Promise.all(
      ALL_TOKENS.map((token) => canvas.findByTestId(`value-${token}`))
    );
    await waitFor(() => {
      const unresolved = ALL_TOKENS.filter(
        (_token, index) => (cells[index].textContent ?? "").trim() === "—"
      );
      expect(unresolved).toEqual([]);
    });

    // Every duration must be zeroed under reduced motion, by a rule that
    // actually targets `:root` — reported as a sorted list so a regression
    // names the survivors rather than failing on whichever token happened to
    // be checked first.
    expect(reducedMotionZeroedTokens().sort()).toEqual([...DURATION_TOKENS].sort());
  },
};
