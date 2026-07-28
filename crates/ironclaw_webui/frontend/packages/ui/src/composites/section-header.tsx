/**
 * SectionHeader / SubLabel
 *
 * Page-level headings shared by list/detail views.
 */
import { cn } from "../primitives/cn";
import { Card } from "../components/card";

/**
 * SectionHeader — top heading card (hidden on mobile, visible md+):
 *   h1 text-[1.9rem] md:text-[2.2rem] font-medium tracking-[-0.04em]
 */
export function SectionHeader({ title, subtitle }) {
  return (
    <Card padding="lg" className="hidden md:block">
      <h1
        className="text-[1.9rem] font-medium tracking-[-0.04em] text-[var(--v2-text-strong)] md:text-[2.2rem]"
      >
        {title}
      </h1>
      {subtitle &&
      (<p className="mt-1 text-[15px] text-[var(--v2-text-muted)]">
        {subtitle}
      </p>)}
    </Card>
  );
}

/**
 * SubLabel — section divider label: text-[1.35rem] font-medium text/82
 */
export function SubLabel({ children, className = "" }) {
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
