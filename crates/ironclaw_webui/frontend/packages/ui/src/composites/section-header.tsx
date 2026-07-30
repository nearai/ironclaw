/**
 * SectionHeader / SubLabel
 *
 * The standard "eyebrow + title + description (+ actions)" arrangement that
 * heads list pages and panels. Promoted from the hand-rolled headers the
 * pages repeated (automations/jobs/routines list heads, detail panels,
 * settings sections) with the token-based eyebrow treatment.
 *
 * Props
 *   eyebrow      mono-caps kicker above the title
 *   title        heading text
 *   titleAs      heading tag, default "h2"
 *   description  muted paragraph under the title
 *   actions      right-aligned controls (filters, refresh, CTAs);
 *                stacks under the text block on small screens
 *   className    layout additions
 *
 * Not boxed — compose inside a Card/Panel when the section needs chrome.
 */
import type { ElementType, ReactNode } from "react";
import { cn } from "../primitives/cn";

type SectionHeaderProps = {
  eyebrow?: ReactNode;
  title?: ReactNode;
  titleAs?: ElementType;
  description?: ReactNode;
  actions?: ReactNode;
  className?: string;
};

export function SectionHeader({
  eyebrow,
  title,
  titleAs: TitleTag = "h2",
  description,
  actions,
  className = "",
}: SectionHeaderProps) {
  return (
    <div
      className={cn(
        "flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between",
        className
      )}
    >
      <div className="min-w-0">
        {eyebrow &&
          (<div className="font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.16em] text-[var(--v2-accent-text)]">
            {eyebrow}
          </div>)}
        {title &&
          (<TitleTag
            className={cn(
              "text-2xl font-semibold tracking-tight text-[var(--v2-text-strong)]",
              eyebrow ? "mt-2" : ""
            )}
          >
            {title}
          </TitleTag>)}
        {description &&
          (<p className="mt-2 max-w-2xl text-sm leading-6 text-[var(--v2-text-muted)]">
            {description}
          </p>)}
      </div>
      {actions &&
        (<div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>)}
    </div>
  );
}

/**
 * SubLabel — section divider label: text-[1.35rem] font-medium text/82
 */
export function SubLabel({ children, className = "" }: { children?: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "mb-4 text-[1.35rem] font-medium text-[var(--v2-text-strong)] opacity-[0.82]",
        className
      )}
    >
      {children}
    </div>
  );
}
