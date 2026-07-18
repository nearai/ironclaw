/**
 * primitives.tsx
 *
 * Higher-level composites that build on Card, Badge, and Button.
 * All existing imports from pages continue to work — nothing was removed.
 *
 * Re-exports: StatusPill (→ Badge), Panel (→ Card)
 * New exports: StatCard, FlowList, EmptyPanel, SectionHeader, SubLabel
 */
import { cn } from "../utils/cn";
import { Card } from "./card";
import { Badge } from "./badge";

/* ── Re-exports ────────────────────────────────────────────────────── */

/** Backwards-compat alias so existing `import { StatusPill }` still works. */
export { Badge, Badge as StatusPill };

/**
 * Panel — thin wrapper over Card so existing `import { Panel }` still works.
 * Usage: <${Panel} className="p-5"> … <//>
 */
export function Panel({ children, className = "", ...rest }) {
  return (<Card className={className} {...rest}>{children}</Card>);
}

/* ── cx helper (kept for any file that imports it from primitives) ── */

export function cx(...classes) {
  return classes.flat().filter(Boolean).join(" ");
}

/* ── StatCard ──────────────────────────────────────────────────────── */
/**
 * A labelled metric card used in summary strips and admin dashboards.
 *
 * Props
 *   label      string
 *   value      string | number
 *   tone       Badge tone
 *   badgeLabel string (optional) — Badge text; defaults to the tone keyword.
 *     Pass a translated label so the chip is not an English tone name.
 *   detail     string (optional sub-text)
 *   showDivider boolean
 *   className  string
 *   valueClassName string (optional) — overrides the value font-size classes.
 *     Defaults to the large numeric size; pass a smaller size for text values
 *     (e.g. a date) that would otherwise truncate. Note: `cn()` only
 *     concatenates (no tailwind-merge), so this REPLACES the size classes
 *     rather than appending to them.
 */
export function StatCard({
  label,
  value,
  tone = "muted",
  badgeLabel = undefined,
  detail = "",
  showDivider = true,
  className = "",
  valueClassName = "text-[1.75rem] md:text-[2rem]",
}) {
  return (
    <div
      className={cn(
        "px-1 py-4",
        showDivider && "border-t border-[var(--v2-panel-border)]",
        className
      )}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div
            className="font-mono text-[0.6875rem] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]"
          >
            {label}
          </div>
          <div
            className={cn(
              "mt-3 truncate font-medium tracking-[-0.05em] text-[var(--v2-text-strong)]",
              valueClassName
            )}
          >
            {value}
          </div>
          {detail &&
          (<div className="mt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
            {detail}
          </div>)}
        </div>
        <Badge tone={tone} label={badgeLabel ?? tone} />
      </div>
    </div>
  );
}

/* ── FlowList ──────────────────────────────────────────────────────── */
/**
 * Numbered list of { title, description } items.
 */
export function FlowList({ items }) {
  return (
    <div className="grid gap-3">
      {items.map(
        (item, index) => (
          <div
            key={item.title}
            className="grid grid-cols-[2.75rem_minmax(0,1fr)] gap-4 border-t border-[var(--v2-panel-border)] py-4"
            style={{ "--index": index } as any}
          >
            <div className="font-mono text-xs text-[var(--v2-accent-text)]">
              {String(index + 1).padStart(2, "0")}
            </div>
            <div className="min-w-0">
              <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                {item.title}
              </div>
              <div className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
                {item.description}
              </div>
            </div>
          </div>
        )
      )}
    </div>
  );
}

/* ── EmptyPanel ────────────────────────────────────────────────────── */
/**
 * Placeholder card shown when a list is empty.
 *
 * Props
 *   title       string
 *   description string
 *   children    optional CTA (usually a Button)
 *   boxed       boolean (wrap in Card)
 */
export function EmptyPanel({ title, description, children = null, boxed = true }) {
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

  if (!boxed) {
    return (<div className="py-8">{body}</div>);
  }

  return (<Card padding="lg">{body}</Card>);
}

/* ── SectionHeader ─────────────────────────────────────────────────── */
/**
 * Top heading card (hidden on mobile, visible md+) matching reference:
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

/* ── SubLabel ──────────────────────────────────────────────────────── */
/**
 * Section divider label: text-[1.35rem] font-medium text/82
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
