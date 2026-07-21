import type { ReactNode } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  COLOR_TOKENS,
  CONTROL_TOKENS,
  MOTION_TOKENS,
  RADIUS_TOKENS,
  SHADOW_TOKENS,
  SPACE_TOKENS,
  STATUS_CANON,
  TYPE_TOKENS,
  Z_TOKENS,
} from "../../src/tokens";
import { Badge } from "../../src/badge";

/* Renders straight from tokens.ts — the machine-readable catalog — so this
   page can never drift from what the package actually ships. Swatches show
   the live computed value under the active theme (use the toolbar toggle). */

const meta = {
  title: "Tokens/Reference",
  parameters: {
    layout: "padded",
    docs: {
      description: {
        component:
          "Every `--v2-*` token the system exposes, rendered live from `tokens.ts`. " +
          "Values are theme-aware — flip the theme toolbar (light / dark / soft) and " +
          "the swatches follow. Import the same catalog in code: " +
          "`import { COLOR_TOKENS } from \"@ironclaw/design-system/tokens\"`.",
      },
    },
  },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

function TokenRow({
  name,
  note,
  preview,
}: {
  name: string;
  note: string;
  preview?: ReactNode;
}) {
  return (
    <div className="flex items-center gap-4 border-b border-[var(--v2-panel-border)] py-2.5 last:border-b-0">
      {preview}
      <code className="w-64 shrink-0 font-mono text-xs text-[var(--v2-accent-text)]">{name}</code>
      <span className="text-sm text-[var(--v2-text-muted)]">{note}</span>
    </div>
  );
}

export const Colors: Story = {
  render: () => (
    <div className="w-[44rem]">
      {COLOR_TOKENS.map((group) => (
        <section key={group.group} className="mb-8">
          <h3 className="v2-tag-face mb-2 text-[length:var(--v2-font-size-label)] text-[var(--v2-text-faint)]">
            {group.group}
          </h3>
          {group.tokens.map((token) => (
            <TokenRow
              key={token.var}
              name={token.var}
              note={token.note}
              preview={
                <span
                  className="h-8 w-8 shrink-0 rounded-[var(--v2-radius-xs)] border border-[var(--v2-panel-border)]"
                  style={{ background: `var(${token.var})` }}
                />
              }
            />
          ))}
        </section>
      ))}
    </div>
  ),
};

export const StatusCanon: Story = {
  name: "Status canon",
  render: () => (
    <div className="w-[44rem]">
      <p className="mb-4 text-sm text-[var(--v2-text-muted)]">
        THE one mapping from product status words to semantic tokens. Text,
        dots, and progress fills always draw from the same pair — never a
        second hue.
      </p>
      {STATUS_CANON.map((entry) => (
        <div
          key={entry.status}
          className="flex items-center gap-4 border-b border-[var(--v2-panel-border)] py-2.5 last:border-b-0"
        >
          <Badge tone={entry.tone as any} label={entry.tone} />
          <span className="w-64 text-sm text-[var(--v2-text)]">{entry.status}</span>
          <code className="font-mono text-xs text-[var(--v2-text-faint)]">
            {entry.text} / {entry.fill}
          </code>
        </div>
      ))}
    </div>
  ),
};

export const Typography: Story = {
  render: () => (
    <div className="w-[44rem]">
      {TYPE_TOKENS.map((token) => (
        <div key={token.var} className="border-b border-[var(--v2-panel-border)] py-3 last:border-b-0">
          <div className="flex items-baseline gap-4">
            <code className="w-72 shrink-0 font-mono text-xs text-[var(--v2-accent-text)]">{token.var}</code>
            <span className="text-xs text-[var(--v2-text-faint)]">{token.note}</span>
          </div>
          <div
            className="mt-1 text-[var(--v2-text-strong)]"
            style={{ fontSize: `var(${token.var})` }}
          >
            {token.sample ?? "IronClaw runs your routine work."}
          </div>
        </div>
      ))}
    </div>
  ),
};

export const RadiusAndSpace: Story = {
  name: "Radius & spacing",
  render: () => (
    <div className="grid w-[44rem] gap-10">
      <section>
        <h3 className="v2-tag-face mb-3 text-[length:var(--v2-font-size-label)] text-[var(--v2-text-faint)]">Radii</h3>
        <div className="flex flex-wrap items-end gap-4">
          {RADIUS_TOKENS.map((token) => (
            <div key={token.var} className="text-center">
              <div
                className="h-16 w-16 border border-[var(--v2-accent)] bg-[var(--v2-accent-soft)]"
                style={{ borderRadius: `var(${token.var})` }}
              />
              <code className="mt-1 block font-mono text-[10px] text-[var(--v2-text-faint)]">
                {token.var.replace("--v2-radius-", "")}
              </code>
            </div>
          ))}
        </div>
      </section>
      <section>
        <h3 className="v2-tag-face mb-3 text-[length:var(--v2-font-size-label)] text-[var(--v2-text-faint)]">Spacing (4px grid)</h3>
        {SPACE_TOKENS.map((token) => (
          <TokenRow
            key={token.var}
            name={token.var}
            note={token.note}
            preview={
              <span
                className="h-3 shrink-0 rounded-sm bg-[var(--v2-accent)]"
                style={{ width: `var(${token.var})` }}
              />
            }
          />
        ))}
      </section>
      <section>
        <h3 className="v2-tag-face mb-3 text-[length:var(--v2-font-size-label)] text-[var(--v2-text-faint)]">Control density</h3>
        {CONTROL_TOKENS.map((token) => (
          <TokenRow key={token.var} name={token.var} note={token.note} />
        ))}
      </section>
    </div>
  ),
};

export const Elevation: Story = {
  render: () => (
    <div className="w-[44rem]">
      <div className="mb-8 flex flex-wrap gap-6">
        {SHADOW_TOKENS.filter((token) => !token.var.includes("accent")).map((token) => (
          <div key={token.var} className="text-center">
            <div
              className="h-20 w-28 rounded-[var(--v2-radius-md)] border border-[var(--v2-card-border)] bg-[var(--v2-card-bg)]"
              style={{ boxShadow: `var(${token.var})` }}
            />
            <code className="mt-2 block font-mono text-[10px] text-[var(--v2-text-faint)]">
              {token.var}
            </code>
          </div>
        ))}
      </div>
      {SHADOW_TOKENS.map((token) => (
        <TokenRow key={token.var} name={token.var} note={token.note} />
      ))}
    </div>
  ),
};

export const MotionAndZ: Story = {
  name: "Motion & z-index",
  render: () => (
    <div className="grid w-[44rem] gap-10">
      <section>
        <h3 className="v2-tag-face mb-3 text-[length:var(--v2-font-size-label)] text-[var(--v2-text-faint)]">Motion</h3>
        <p className="mb-3 text-sm text-[var(--v2-text-muted)]">
          Purposeful and quick: pick a duration + easing pair, never a raw ms
          value. Entrances ease out; exits are quicker than entrances. All
          motion is suppressed under prefers-reduced-motion.
        </p>
        {MOTION_TOKENS.map((token) => (
          <TokenRow key={token.var} name={token.var} note={token.note} />
        ))}
      </section>
      <section>
        <h3 className="v2-tag-face mb-3 text-[length:var(--v2-font-size-label)] text-[var(--v2-text-faint)]">Z-index ladder</h3>
        {Z_TOKENS.map((token) => (
          <TokenRow key={token.var} name={token.var} note={token.note} />
        ))}
      </section>
    </div>
  ),
};
