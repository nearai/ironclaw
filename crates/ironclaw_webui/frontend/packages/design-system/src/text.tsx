/**
 * Text / Heading — the typography primitives.
 *
 * One place where font-size / weight / color combinations are decided, so
 * pages stop re-deriving `text-xs text-[var(--v2-text-muted)]` by hand.
 * Variants map 1:1 onto the TYPE_TOKENS scale (tokens.ts / tokens.css);
 * tones map onto the semantic text colors. Line heights follow the same
 * pairings the workspace already uses (body copy at a relaxed leading,
 * captions/meta tighter).
 *
 *   <Text variant="caption" tone="muted">Last synced 2 minutes ago</Text>
 *   <Text variant="eyebrow" tone="accent">Admin</Text>
 *   <Heading level={2}>Recent activity</Heading>
 *
 * `as` picks the rendered element (default: p for body variants, span for
 * the small ones). Heading renders h1–h6 with the display/heading/title
 * steps of the scale.
 */
import { cva, type VariantProps } from "class-variance-authority";
import { createElement, type ElementType, type ReactNode } from "react";
import { cn } from "./cn";

const textVariants = cva("", {
  variants: {
    variant: {
      /** 36px — page h1 (desktop). */
      display:
        "text-[length:var(--v2-font-size-display)] font-medium tracking-[var(--v2-tracking-display)] leading-tight",
      /** 28px — stat values, hero numbers. */
      "display-sm":
        "text-[length:var(--v2-font-size-display-sm)] font-medium tracking-[var(--v2-tracking-tight)] leading-tight",
      /** 24px — section headings. */
      heading:
        "text-[length:var(--v2-font-size-heading)] font-medium tracking-[var(--v2-tracking-tight)] leading-snug",
      /** 20px — modal/panel titles. */
      title:
        "text-[length:var(--v2-font-size-title)] font-medium tracking-[var(--v2-tracking-tight)] leading-snug",
      /** 16px — descriptions, empty states. */
      "body-lg": "text-[length:var(--v2-font-size-body-lg)] leading-7",
      /** 14px — body copy (desktop). */
      body: "text-[length:var(--v2-font-size-body)] leading-6",
      /** 13px — body copy (mobile) + control labels. */
      "body-sm": "text-[length:var(--v2-font-size-body-sm)] leading-6",
      /** 12px — hints, meta rows, errors. */
      caption: "text-[length:var(--v2-font-size-caption)] leading-5",
      /** 11px mono uppercase — the panel/card eyebrow label. */
      eyebrow:
        "font-mono text-[length:var(--v2-font-size-label)] uppercase tracking-[var(--v2-tracking-caps)]",
      /** 11px pixel tag face — badges, tags (v2-tag-face handles caps). */
      label: "v2-tag-face text-[length:var(--v2-font-size-label)]",
      /** Mono data — ids, durations, counts. Tabular so columns align. */
      mono: "font-mono text-[length:var(--v2-font-size-caption)] tabular-nums",
    },
    tone: {
      /** Default body ink. */
      default: "text-[var(--v2-text)]",
      strong: "text-[var(--v2-text-strong)]",
      muted: "text-[var(--v2-text-muted)]",
      faint: "text-[var(--v2-text-faint)]",
      accent: "text-[var(--v2-accent-text)]",
      positive: "text-[var(--v2-positive-text)]",
      warning: "text-[var(--v2-warning-text)]",
      danger: "text-[var(--v2-danger-text)]",
      info: "text-[var(--v2-info-text)]",
      /** Inherit the parent's color (compose inside toned containers). */
      inherit: "",
    },
    weight: {
      inherit: "",
      normal: "font-normal",
      medium: "font-medium",
      /** Weight ceiling — browser-default bold reads too heavy in Geist. */
      semibold: "font-semibold",
    },
  },
  defaultVariants: {
    variant: "body",
    tone: "default",
    weight: "inherit",
  },
});

export type TextVariant = NonNullable<VariantProps<typeof textVariants>["variant"]>;
export type TextTone = NonNullable<VariantProps<typeof textVariants>["tone"]>;
export type TextWeight = NonNullable<VariantProps<typeof textVariants>["weight"]>;

/* Body-shaped variants default to <p>; inline/meta variants to <span>. */
const DEFAULT_ELEMENT: Record<TextVariant, ElementType> = {
  display: "p",
  "display-sm": "p",
  heading: "p",
  title: "p",
  "body-lg": "p",
  body: "p",
  "body-sm": "p",
  caption: "span",
  eyebrow: "span",
  label: "span",
  mono: "span",
};

export interface TextProps {
  variant?: TextVariant;
  tone?: TextTone;
  weight?: TextWeight;
  /** Rendered element; defaults per variant (p for body, span for meta). */
  as?: ElementType;
  className?: string;
  children?: ReactNode;
  [key: string]: unknown;
}

export function Text({
  variant = "body",
  tone = "default",
  weight = "inherit",
  as,
  className = "",
  children,
  ...rest
}: TextProps) {
  const element = as ?? DEFAULT_ELEMENT[variant] ?? "span";
  return createElement(
    element,
    { className: cn(textVariants({ variant, tone, weight }), className), ...rest },
    children
  );
}

/* ─── Heading ─────────────────────────────────────────────────────── */

const HEADING_VARIANT: Record<number, TextVariant> = {
  1: "display",
  2: "heading",
  3: "title",
  4: "body-lg",
  5: "body",
  6: "body-sm",
};

export interface HeadingProps {
  /** Semantic level; also picks the type-scale step (1→display … 3→title). */
  level?: 1 | 2 | 3 | 4 | 5 | 6;
  /** Override the visual step without changing the semantic element. */
  variant?: TextVariant;
  tone?: TextTone;
  weight?: TextWeight;
  className?: string;
  children?: ReactNode;
  [key: string]: unknown;
}

export function Heading({
  level = 2,
  variant,
  tone = "strong",
  weight = "inherit",
  className = "",
  children,
  ...rest
}: HeadingProps) {
  return (
    <Text
      as={`h${level}` as ElementType}
      variant={variant ?? HEADING_VARIANT[level]}
      tone={tone}
      weight={weight}
      className={className}
      {...rest}
    >
      {children}
    </Text>
  );
}
