/**
 * CodePanel
 *
 * Read-only mono block for payload dumps, file previews and device codes.
 * Consolidates the page-level `<pre>` variants (job files, workspace viewer,
 * routine payloads, approval previews) onto one token surface. Not a code
 * *editor* and not the chat markdown code block — those stay bespoke.
 *
 * Props
 *   wrap       soft-wrap long lines instead of horizontal scrolling
 *   className  sizing additions (e.g. "max-h-[60vh]")
 *   ...rest    forwarded to the <pre> (data-testid, …)
 */
import type { ComponentPropsWithoutRef, ReactNode } from "react";
import { cn } from "../primitives/cn";

type CodePanelProps = {
  wrap?: boolean;
  children?: ReactNode;
  className?: string;
} & Omit<ComponentPropsWithoutRef<"pre">, "className" | "children">;

export function CodePanel({ wrap = false, children, className = "", ...rest }: CodePanelProps) {
  return (
    <pre
      className={cn(
        "rounded-[12px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-muted)]",
        "p-4 font-mono text-xs leading-5 text-[var(--v2-text)]",
        wrap ? "whitespace-pre-wrap [overflow-wrap:anywhere]" : "overflow-x-auto",
        "overflow-y-auto",
        className
      )}
      {...rest}
    >
      {children}
    </pre>
  );
}
