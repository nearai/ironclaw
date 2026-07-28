/**
 * EmptyPanel
 *
 * Placeholder card shown when a list is empty.
 *
 * Props
 *   title       string
 *   description string
 *   children    optional CTA (usually a Button)
 *   boxed       boolean (wrap in Card)
 */
import { Card } from "../components/card";

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
