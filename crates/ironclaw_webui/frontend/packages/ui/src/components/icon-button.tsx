/**
 * IconButton
 *
 * The compact 32×32 icon control used across the app chrome (header
 * actions, sidebar footer, notification bell). Extracted from the repeated
 * `grid h-8 w-8 place-items-center rounded-[8px] …` pattern.
 *
 * Props
 *   variant   "ghost" (default) — muted glyph, surface hover
 *             "plain"           — layout only; caller supplies colors
 *   active    boolean — accent-tinted active/selected state
 *   as        "button" (default) | "a" | Link-like component
 *   className extra classes (e.g. "relative", custom colors for "plain")
 *   ...rest   forwarded (onClick, aria-*, data-*, title, ref, …)
 *
 * `iconButtonClasses` is exported for call sites that must build the class
 * string themselves (e.g. react-router <NavLink className={fn}> callbacks).
 */
import type { ElementType, ReactNode } from "react";
import { cn } from "../primitives/cn";

const BASE = "grid h-8 w-8 shrink-0 place-items-center rounded-[8px]";

const VARIANTS = {
  ghost:
    "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
  plain: "",
};

const ACTIVE = "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]";

type IconButtonStyleOptions = {
  variant?: keyof typeof VARIANTS;
  active?: boolean;
  className?: string;
};

export function iconButtonClasses({
  variant = "ghost",
  active = false,
  className = "",
}: IconButtonStyleOptions = {}) {
  return cn(BASE, VARIANTS[variant] ?? VARIANTS.ghost, active && ACTIVE, className);
}

type IconButtonProps = IconButtonStyleOptions & {
  children?: ReactNode;
  as?: ElementType;
  type?: string;
  [key: string]: unknown;
};

export function IconButton({
  children,
  className = "",
  variant = "ghost",
  active = false,
  as: Tag = "button",
  type,
  ...rest
}: IconButtonProps) {
  const Element: any = Tag;
  const resolvedType = Tag === "button" ? type ?? "button" : type;
  return (
    <Element
      type={resolvedType}
      className={iconButtonClasses({ variant, active, className })}
      {...rest}
    >
      {children}
    </Element>
  );
}
