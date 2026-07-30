/**
 * Toolbar
 *
 * Layout shell for the "search + filters + actions" row that heads list
 * views. Stacks on small screens and lays out in a row from md up; give the
 * search control `className="md:flex-1"` (or wrap groups in ToolbarGroup) so
 * it absorbs the leftover width. Pure rhythm — the controls themselves are
 * SearchInput / Select / SegmentedControl / Button.
 */
import type { ReactNode } from "react";
import { cn } from "../primitives/cn";

type ToolbarProps = {
  children?: ReactNode;
  className?: string;
};

export function Toolbar({ children, className = "" }: ToolbarProps) {
  return (
    <div className={cn("flex flex-col gap-3 md:flex-row md:items-center", className)}>
      {children}
    </div>
  );
}

/** Groups trailing controls so they wrap together on small screens. */
export function ToolbarGroup({ children, className = "" }: ToolbarProps) {
  return (
    <div className={cn("flex shrink-0 flex-wrap items-center gap-2", className)}>
      {children}
    </div>
  );
}
