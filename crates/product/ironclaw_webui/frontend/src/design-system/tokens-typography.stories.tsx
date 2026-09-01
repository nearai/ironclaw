import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import { expect, waitFor } from "storybook/test";

/**
 * Font families and the type ramp from `src/styles/app.css`.
 *
 * Every printed size is READ from the live custom property rather than copied
 * into this file. A duplicated literal here would keep displaying `1.125rem`
 * after Phase 3a (#7781 WS3) retunes `--text-title`, so the sheet would drift
 * from the contract it documents — the same live-sheet rule the shape sheet
 * next door follows.
 */

const FONTS = [
  { token: "--font-sans", label: "Sans — Roboto Flex" },
  { token: "--font-serif", label: "Serif — Newsreader" },
  { token: "--font-mono", label: "Mono — Roboto Mono" },
];

// Control text — labels, buttons, form copy.
const SCALE = [
  { token: "--text-ui-xs", label: "text-ui-xs" },
  { token: "--text-ui-sm", label: "text-ui-sm" },
  { token: "--text-ui", label: "text-ui" },
  { token: "--text-ui-lg", label: "text-ui-lg" },
];

// Editorial text — headings and display copy, a tier the ramp could not
// express before. 539 of the app's ~618 type usages are still raw `text-xs` /
// `text-sm` against 21 on the semantic ramp; these steps are what the rest
// migrate onto, and what Phase 3a (#7781 WS3) retunes when it takes the
// density decision.
const EDITORIAL = [
  { token: "--text-title-sm", label: "text-title-sm" },
  { token: "--text-title", label: "text-title" },
  { token: "--text-title-lg", label: "text-title-lg" },
  { token: "--text-display", label: "text-display" },
];

/** Every token this sheet documents, for the `play` resolution check. */
const ALL_TOKENS = [...FONTS, ...SCALE, ...EDITORIAL].map(({ token }) => token);

const PANGRAM = "The quick brown fox jumps over the lazy dog";

function useResolved(token: string): string {
  const [resolved, setResolved] = useState("");
  useEffect(() => {
    setResolved(getComputedStyle(document.documentElement).getPropertyValue(token).trim());
  }, [token]);
  return resolved;
}

function FontRow({ token, label }: { token: string; label: string }) {
  const resolved = useResolved(token);
  return (
    <div>
      <div className="font-mono text-[0.625rem] text-[var(--v2-text-muted)]">{token}</div>
      <div className="text-title" style={{ fontFamily: `var(${token})` }}>
        {PANGRAM}
      </div>
      <div className="text-ui-sm text-[var(--v2-text-muted)]">{label}</div>
      <div
        data-testid={`value-${token}`}
        className="truncate font-mono text-[0.625rem] text-[var(--v2-text-faint)]"
      >
        {resolved || "—"}
      </div>
    </div>
  );
}

function SizeRow({ token, label }: { token: string; label: string }) {
  const resolved = useResolved(token);
  return (
    <div className="flex flex-wrap items-baseline gap-3">
      <span style={{ fontSize: `var(${token})` }}>{PANGRAM}</span>
      <span className="font-mono text-[0.625rem] text-[var(--v2-text-muted)]">
        {label} · <span data-testid={`value-${token}`}>{resolved || "—"}</span>
      </span>
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <h3 className="mb-3 font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]">
        {title}
      </h3>
      <div className="flex flex-col gap-3">{children}</div>
    </section>
  );
}

const meta = {
  title: "Tokens/Typography",
} satisfies Meta;

export default meta;
type Story = StoryObj;

export const Typography: Story = {
  render: () => (
    <div className="flex flex-col gap-8 text-[var(--v2-text-strong)]">
      <Section title="Font families">
        {FONTS.map(({ token, label }) => (
          <FontRow key={token} token={token} label={label} />
        ))}
      </Section>
      <Section title="UI type scale">
        {SCALE.map(({ token, label }) => (
          <SizeRow key={token} token={token} label={label} />
        ))}
      </Section>
      <Section title="Editorial type scale">
        {EDITORIAL.map(({ token, label }) => (
          <SizeRow key={token} token={token} label={label} />
        ))}
      </Section>
    </div>
  ),
  // Rendering alone cannot prove the stylesheet loaded: a dropped or renamed
  // token yields an empty string and the sample falls back to a browser
  // default that still looks like text. Assert each one resolves, polling
  // because `useResolved` fills the cell in a passive effect.
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
  },
};
