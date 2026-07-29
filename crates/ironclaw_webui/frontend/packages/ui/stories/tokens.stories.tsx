import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";

const COLOR_TOKENS = [
  "--v2-canvas", "--v2-canvas-strong", "--v2-surface", "--v2-surface-soft",
  "--v2-surface-muted", "--v2-card-bg", "--v2-panel-border", "--v2-input-bg",
  "--v2-text", "--v2-text-strong", "--v2-text-muted", "--v2-text-faint",
  "--v2-accent", "--v2-accent-strong", "--v2-accent-soft", "--v2-accent-text",
  "--v2-positive-soft", "--v2-positive-text", "--v2-warning-soft",
  "--v2-warning-text", "--v2-danger-soft", "--v2-danger-text",
  "--v2-info-soft", "--v2-info-text", "--v2-focus-ring",
];

const TYPE_TOKENS = [
  ["--text-ui-sm", "0.75rem"],
  ["--text-ui", "0.8125rem"],
  ["--text-ui-lg", "1rem"],
];

const meta: Meta = { title: "Tokens/Overview" };
export default meta;

export const Colors: StoryObj = {
  render: () => (
    <div className="grid grid-cols-4 gap-3">
      {COLOR_TOKENS.map((token) => (
        <div
          key={token}
          className="flex flex-col gap-1.5 rounded-[10px] border border-[var(--v2-panel-border)] p-2"
        >
          <div
            className="h-10 rounded-[6px] border border-[var(--v2-panel-border)]"
            style={{ background: `var(${token})` }}
          />
          <span className="font-mono text-[10px] text-[var(--v2-text-muted)]">{token}</span>
        </div>
      ))}
    </div>
  ),
};

export const TypeScale: StoryObj = {
  render: () => (
    <div className="grid gap-3">
      {TYPE_TOKENS.map(([token, size]) => (
        <div key={token} className="flex items-baseline gap-4">
          <span className="w-32 font-mono text-[10px] text-[var(--v2-text-faint)]">
            {token} · {size}
          </span>
          <span style={{ fontSize: `var(${token})` }} className="text-[var(--v2-text)]">
            Shared control typography
          </span>
        </div>
      ))}
    </div>
  ),
};
