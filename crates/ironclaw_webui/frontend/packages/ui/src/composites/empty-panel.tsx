/**
 * EmptyPanel
 *
 * Placeholder shown when a list or region has nothing to display.
 *
 * Props
 *   variant     "card" (default) — full empty state inside a Card
 *               "plain"          — same typography, no chrome
 *               "dashed"         — compact dashed drop-zone placeholder for
 *                                  inline regions (columns, sub-panels)
 *   title       heading (optional for dashed placeholders)
 *   description supporting copy
 *   children    optional CTA (usually a Button)
 */
import type { ReactNode } from "react";
import { cn } from "../primitives/cn";
import { Card } from "../components/card";

type EmptyPanelProps = {
  title?: ReactNode;
  description?: ReactNode;
  children?: ReactNode;
  variant?: "card" | "plain" | "dashed";
  className?: string;
};

export function EmptyPanel({
  title,
  description,
  children = null,
  variant = "card",
  className = "",
}: EmptyPanelProps) {
  if (variant === "dashed") {
    return (
      <div
        className={cn(
          "rounded-[16px] border border-dashed border-[var(--v2-panel-border)] px-4 py-6",
          className
        )}
      >
        {title &&
          (<div className="text-sm font-medium text-[var(--v2-text-strong)]">{title}</div>)}
        {description &&
          (<p className={cn("text-sm leading-6 text-[var(--v2-text-muted)]", title ? "mt-1" : "")}>
            {description}
          </p>)}
        {children && (<div className="mt-3">{children}</div>)}
      </div>
    );
  }

  const body = (
    <div className="max-w-xl">
      <h2
        className="text-[1.35rem] font-medium tracking-[-0.03em] text-[var(--v2-text-strong)] md:text-[1.6rem]"
      >
        {title}
      </h2>
      <p className="mt-3 text-[15px] leading-relaxed text-[var(--v2-text-muted)]">
        {description}
      </p>
      {children && (<div className="mt-5">{children}</div>)}
    </div>
  );

  if (variant === "plain") {
    return (<div className={cn("py-8", className)}>{body}</div>);
  }

  return (<Card padding="lg" className={className}>{body}</Card>);
}
