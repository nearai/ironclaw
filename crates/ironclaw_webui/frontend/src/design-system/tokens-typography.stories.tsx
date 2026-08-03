import type { Meta, StoryObj } from "@storybook/react-vite";

const FONTS = [
  { token: "--font-sans", label: "Sans — Geist" },
  { token: "--font-serif", label: "Serif — Newsreader" },
  { token: "--font-mono", label: "Mono — Geist Mono" },
];

const SCALE = [
  { token: "--text-ui-sm", label: "text-ui-sm", size: "0.75rem" },
  { token: "--text-ui", label: "text-ui", size: "0.8125rem" },
  { token: "--text-ui-lg", label: "text-ui-lg", size: "1rem" },
];

const PANGRAM = "The quick brown fox jumps over the lazy dog";

const meta = {
  title: "Tokens/Typography",
} satisfies Meta;

export default meta;
type Story = StoryObj;

export const Typography: Story = {
  render: () => (
    <div className="flex flex-col gap-8 text-[var(--v2-text-strong)]">
      <section>
        <h3 className="mb-3 font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]">
          Font families
        </h3>
        <div className="flex flex-col gap-4">
          {FONTS.map(({ token, label }) => (
            <div key={token}>
              <div className="font-mono text-[0.625rem] text-[var(--v2-text-muted)]">{token}</div>
              <div className="text-xl" style={{ fontFamily: `var(${token})` }}>{PANGRAM}</div>
              <div className="text-xs text-[var(--v2-text-muted)]">{label}</div>
            </div>
          ))}
        </div>
      </section>
      <section>
        <h3 className="mb-3 font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]">
          UI type scale
        </h3>
        <div className="flex flex-col gap-3">
          {SCALE.map(({ token, label, size }) => (
            <div key={token} className="flex flex-wrap items-baseline gap-3">
              <span style={{ fontSize: `var(${token})` }}>{PANGRAM}</span>
              <span className="font-mono text-[0.625rem] text-[var(--v2-text-muted)]">
                {label} · {size}
              </span>
            </div>
          ))}
        </div>
      </section>
    </div>
  ),
};
