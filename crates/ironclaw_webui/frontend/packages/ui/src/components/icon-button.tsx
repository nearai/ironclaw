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
 *   ...rest   forwarded; typed against the rendered element, so only the
 *             attributes valid for `as` are accepted (href on "a", …)
 *
 * `iconButtonClasses` is exported for call sites that must build the class
 * string themselves (e.g. react-router <NavLink className={fn}> callbacks).
 */
import type { ComponentProps, ElementType, ReactNode } from "react";
import { cn } from "../primitives/cn";

const BASE =
  "grid h-8 w-8 shrink-0 place-items-center rounded-[8px] transition-colors duration-150 " +
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)] " +
  "disabled:cursor-not-allowed disabled:opacity-50 disabled:pointer-events-none";

const VARIANTS = {
  ghost:
    "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] " +
    "active:bg-[color-mix(in_srgb,var(--v2-text-strong)_10%,var(--v2-surface-muted))]",
  plain: "",
};

/* Selected state carries its own hover/active steps so the accent tint still
   reads under the pointer (the ghost hover classes are skipped when active). */
const ACTIVE =
  "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)] " +
  "hover:bg-[color-mix(in_srgb,var(--v2-accent)_18%,transparent)] " +
  "active:bg-[color-mix(in_srgb,var(--v2-accent)_24%,transparent)]";

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
  const variantClass =
    active && variant === "ghost" ? "" : (VARIANTS[variant] ?? VARIANTS.ghost);
  return cn(BASE, variantClass, active && ACTIVE, className);
}

type IconButtonOwnProps<E extends ElementType> = IconButtonStyleOptions & {
  as?: E;
  children?: ReactNode;
};

export type IconButtonProps<E extends ElementType = "button"> = IconButtonOwnProps<E> &
  Omit<ComponentProps<E>, keyof IconButtonOwnProps<E>>;

export function IconButton<E extends ElementType = "button">({
  children,
  className = "",
  variant = "ghost",
  active = false,
  as,
  ...rest
}: IconButtonProps<E>) {
  const Element = (as ?? "button") as ElementType<Record<string, unknown>>;
  // Native buttons default to type="submit"; keep icon buttons inert unless
  // the caller opts in.
  const defaultedProps =
    Element === "button" && (rest as { type?: string }).type === undefined
      ? { ...rest, type: "button" }
      : rest;
  return (
    <Element
      className={iconButtonClasses({ variant, active, className })}
      {...defaultedProps}
    >
      {children}
    </Element>
  );
}
