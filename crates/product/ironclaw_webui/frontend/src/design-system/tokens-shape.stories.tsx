import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import { expect, waitFor } from "storybook/test";

/**
 * Shape, space, elevation and stacking tokens from `src/styles/app.css`.
 *
 * The colour sheet next door proves the palette resolves; until this sheet
 * existed nothing proved the same for the non-colour axes, which is how six
 * competing corner radii and 23 arbitrary `shadow-[…]` accumulated unnoticed.
 *
 * Every sample reads the live `var(--token)` rather than a copied literal, so
 * when Phase 3a (#7781 WS3) rewrites the values this page shows the new scale
 * with no edit — and the `play` assertion below fails loudly if a token is ever
 * renamed out from under a consumer.
 */

const RADII = [
  ["--v2-radius-chip", "badges, dots, inline marks"],
  ["--v2-radius-field", "inputs, selects, textareas"],
  ["--v2-radius-control-sm", "small + icon-sm buttons"],
  ["--v2-radius-control", "default buttons, below md:"],
  ["--v2-radius-control-lg", "default buttons, md: and up"],
  ["--v2-radius-control-xl", "large buttons"],
  ["--v2-radius-surface", "cards, panels"],
  ["--v2-radius-surface-lg", "modals, sheets"],
  ["--v2-radius-pill", "avatars, toggles, pills"],
] as const;

const SPACES = [
  ["--v2-space-gutter", "page gutter"],
  ["--v2-space-inset-sm", "tight component padding"],
  ["--v2-space-inset", "default component padding"],
  ["--v2-space-inset-lg", "roomy component padding"],
  ["--v2-space-stack-sm", "tight sibling gap"],
  ["--v2-space-stack", "default sibling gap"],
  ["--v2-space-stack-lg", "section gap"],
] as const;

const ELEVATIONS = [
  ["--v2-elevation-0", "flush with the ground"],
  ["--v2-elevation-1", "resting surface — cards, panels"],
  ["--v2-elevation-2", "popovers, menus, dropdowns"],
  ["--v2-elevation-3", "modals, sheets"],
] as const;

const LAYERS = [
  ["--v2-z-base", "document flow"],
  ["--v2-z-sticky", "sticky headers, rails"],
  ["--v2-z-dropdown", "dropdowns, select menus"],
  ["--v2-z-overlay", "scrims"],
  ["--v2-z-modal", "modals, sheets"],
  ["--v2-z-popover", "popovers above a modal"],
  ["--v2-z-toast", "toasts"],
  ["--v2-z-tooltip", "tooltips — the top layer"],
] as const;

/** Every token this sheet documents, for the `play` resolution check. */
const ALL_TOKENS = [...RADII, ...SPACES, ...ELEVATIONS, ...LAYERS].map(([token]) => token);

function useResolved(token: string, theme: string): string {
  const [resolved, setResolved] = useState("");
  // `data-theme` swaps the elevation steps, so the printed value has to refresh
  // with the toolbar the same way the colour swatches do.
  useEffect(() => {
    setResolved(getComputedStyle(document.documentElement).getPropertyValue(token).trim());
  }, [token, theme]);
  return resolved;
}

function Row({
  token,
  note,
  theme,
  sample,
}: {
  token: string;
  note: string;
  theme: string;
  sample: ReactNode;
}) {
  const resolved = useResolved(token, theme);
  return (
    <div className="flex items-center gap-4 rounded-[12px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-3">
      <span className="flex h-12 w-20 shrink-0 items-center justify-center">{sample}</span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-mono text-xs text-[var(--v2-text-strong)]">
          {token}
        </span>
        <span className="block truncate text-[0.6875rem] text-[var(--v2-text-muted)]">{note}</span>
      </span>
      <span
        data-testid={`value-${token}`}
        className="shrink-0 font-mono text-[0.625rem] text-[var(--v2-text-muted)]"
      >
        {resolved || "—"}
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
      <div className="flex flex-col gap-2">{children}</div>
    </section>
  );
}

const meta = {
  title: "Tokens/Shape & Space",
} satisfies Meta;

export default meta;
type Story = StoryObj;

export const ShapeAndSpace: Story = {
  render: (_args, { globals }) => {
    const theme = globals.theme === "light" ? "light" : "dark";
    return (
      <div className="flex max-w-3xl flex-col gap-8">
        <Section title="Radius">
          {RADII.map(([token, note]) => (
            <Row
              key={token}
              token={token}
              note={note}
              theme={theme}
              sample={
                <span
                  className="h-11 w-16 border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]"
                  style={{ borderRadius: `var(${token})` }}
                />
              }
            />
          ))}
        </Section>

        <Section title="Spacing">
          {SPACES.map(([token, note]) => (
            <Row
              key={token}
              token={token}
              note={note}
              theme={theme}
              sample={
                <span className="flex h-11 w-16 items-center">
                  <span
                    className="h-4 bg-[var(--v2-accent)]"
                    style={{ width: `var(${token})` }}
                  />
                </span>
              }
            />
          ))}
        </Section>

        <Section title="Elevation">
          {ELEVATIONS.map(([token, note]) => (
            <Row
              key={token}
              token={token}
              note={note}
              theme={theme}
              sample={
                <span
                  className="h-11 w-16 rounded-[var(--v2-radius-surface)] border border-[var(--v2-card-border)] bg-[var(--v2-card-bg)]"
                  style={{ boxShadow: `var(${token})` }}
                />
              }
            />
          ))}
        </Section>

        <Section title="Stacking order">
          {LAYERS.map(([token, note]) => (
            <Row
              key={token}
              token={token}
              note={note}
              theme={theme}
              sample={
                <span className="font-mono text-xs text-[var(--v2-text-faint)]">z</span>
              }
            />
          ))}
        </Section>
      </div>
    );
  },
  // The render alone cannot prove the stylesheet loaded: a missing or renamed
  // token yields an empty string and every sample silently falls back to the
  // browser default, which still looks plausible. Assert each one resolves.
  play: async ({ canvas }) => {
    const cells = await Promise.all(
      ALL_TOKENS.map((token) => canvas.findByTestId(`value-${token}`))
    );
    // Poll rather than read once. Every cell exists from the first render
    // holding the "—" placeholder, and `useResolved` fills it in a passive
    // effect — so a single read can catch the pre-effect frame and report
    // every token as missing when nothing is wrong. Retry until they settle.
    //
    // Reported as a list so a rename shows every casualty in one run instead
    // of failing on the first.
    await waitFor(() => {
      const unresolved = ALL_TOKENS.filter(
        (_token, index) => (cells[index].textContent ?? "").trim() === "—"
      );
      expect(unresolved).toEqual([]);
    });
  },
};
