/**
 * DocCallout — docs-only callout used by the guidance pages (Brand
 * principles, Voice & copy) to flag that the content is a strong draft,
 * not settled law. Styled with the same `--v2-*` tokens the system uses
 * so it reads as part of the family; lives in stories/ because it is a
 * Storybook affordance, not a shipped component.
 */
import type { ReactNode } from "react";
import { Icon } from "../../src/icons";

const TONES = {
  info: {
    border: "color-mix(in srgb, var(--v2-info-text) 30%, var(--v2-panel-border))",
    background: "var(--v2-info-soft)",
    text: "var(--v2-info-text)",
    icon: "flag" as const,
  },
  accent: {
    border: "color-mix(in srgb, var(--v2-accent-text) 30%, var(--v2-panel-border))",
    background: "var(--v2-accent-soft)",
    text: "var(--v2-accent-text)",
    icon: "spark" as const,
  },
};

export function DocCallout({
  tone = "info",
  title,
  children,
}: {
  tone?: keyof typeof TONES;
  title: string;
  children: ReactNode;
}) {
  const t = TONES[tone] ?? TONES.info;
  return (
    <aside
      style={{
        display: "grid",
        gridTemplateColumns: "1.25rem minmax(0, 1fr)",
        gap: "0.75rem",
        margin: "1.5rem 0",
        padding: "1rem 1.25rem",
        borderRadius: "var(--v2-radius-lg, 12px)",
        border: `1px solid ${t.border}`,
        background: t.background,
      }}
    >
      <span style={{ width: "1.25rem", marginTop: 3, color: t.text }}>
        <Icon name={t.icon} className="h-[1.05rem] w-[1.05rem]" />
      </span>
      <div style={{ fontSize: "0.875rem", lineHeight: 1.6, color: "var(--v2-text-strong)" }}>
        <strong style={{ display: "block", marginBottom: 2, color: t.text }}>{title}</strong>
        {children}
      </div>
    </aside>
  );
}
