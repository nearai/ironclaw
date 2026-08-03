import type { Meta, StoryObj } from "@storybook/react-vite";
import { useEffect, useState } from "react";

// Color design tokens from src/styles/app.css. The swatch background reads the
// live `var(--token)` so the light/dark toolbar recolors them in place.
const GROUPS: { group: string; tokens: string[] }[] = [
  {
    group: "Canvas & surface",
    tokens: [
      "--v2-canvas", "--v2-canvas-strong", "--v2-surface", "--v2-surface-soft",
      "--v2-surface-muted", "--v2-rail", "--v2-input-bg",
    ],
  },
  {
    group: "Text",
    tokens: ["--v2-text", "--v2-text-strong", "--v2-text-muted", "--v2-text-faint", "--v2-inverse"],
  },
  {
    group: "Accent",
    tokens: ["--v2-accent", "--v2-accent-strong", "--v2-accent-soft", "--v2-accent-text"],
  },
  {
    group: "Semantic",
    tokens: [
      "--v2-positive-soft", "--v2-positive-text", "--v2-warning-soft", "--v2-warning-text",
      "--v2-danger-soft", "--v2-danger-text", "--v2-info-soft", "--v2-info-text",
    ],
  },
  {
    group: "Card & border",
    tokens: ["--v2-card-bg", "--v2-card-border", "--v2-panel-border"],
  },
];

function Swatch({ token }: { token: string }) {
  const [resolved, setResolved] = useState("");
  useEffect(() => {
    setResolved(getComputedStyle(document.documentElement).getPropertyValue(token).trim());
  }, [token]);
  return (
    <div className="flex items-center gap-3 rounded-[12px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-3">
      <span
        className="h-9 w-9 shrink-0 rounded-[8px] border border-[var(--v2-panel-border)]"
        style={{ background: `var(${token})` }}
      />
      <span className="min-w-0">
        <span className="block truncate font-mono text-xs text-[var(--v2-text-strong)]">{token}</span>
        <span className="block truncate font-mono text-[0.625rem] text-[var(--v2-text-muted)]">
          {resolved || "—"}
        </span>
      </span>
    </div>
  );
}

const meta = {
  title: "Tokens/Colors",
} satisfies Meta;

export default meta;
type Story = StoryObj;

export const Colors: Story = {
  render: () => (
    <div className="flex flex-col gap-6">
      {GROUPS.map(({ group, tokens }) => (
        <section key={group}>
          <h3 className="mb-3 font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]">
            {group}
          </h3>
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
            {tokens.map((token) => (
              <Swatch key={token} token={token} />
            ))}
          </div>
        </section>
      ))}
    </div>
  ),
};
