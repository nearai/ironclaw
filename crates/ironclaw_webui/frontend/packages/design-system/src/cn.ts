/**
 * Class-name merger (shadcn pattern).
 *
 * Accepts any mix of strings, arrays, objects ({ "cls": bool }), and falsy
 * values — returns a single space-separated class string with Tailwind
 * conflicts resolved via tailwind-merge.
 *
 * tailwind-merge is extended with the semantic control type scale
 * (`text-ui-sm` / `text-ui` / `text-ui-lg`, defined in tokens.css @theme):
 * without this, the default config can't tell them apart from text COLOR
 * utilities like `text-[var(--v2-text-strong)]`, and one silently removes
 * the other.
 */
import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

const twMerge = extendTailwindMerge({
  extend: {
    classGroups: {
      "font-size": ["text-ui-sm", "text-ui", "text-ui-lg"],
    },
  },
});

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
