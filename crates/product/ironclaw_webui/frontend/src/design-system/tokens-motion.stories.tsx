import type { Meta, StoryObj } from "@storybook/react-vite";

import { Spinner } from "./spinner";

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
          <div className="font-mono text-xs text-[var(--v2-text-strong)]">.v2-spin</div>
          <div className="text-xs text-[var(--v2-text-muted)]">
            0.8s linear infinite — the one always-on animation
          </div>
        </div>
      </div>
      <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
        The app ships a static-motion policy: a global{" "}
        <code className="rounded bg-[var(--v2-surface-soft)] px-1 font-mono text-xs">
          {"* { animation: none !important }"}
        </code>{" "}
        rule disables transitions and animations by default. Only a few class-scoped exceptions
        (notably <code className="font-mono text-xs">.v2-spin</code> for loading spinners) opt back
        in, and <code className="font-mono text-xs">prefers-reduced-motion</code> re-suppresses even
        those.
      </p>
    </div>
  ),
};
