/**
 * Class-name merger (shadcn pattern).
 *
 * Accepts any mix of strings, arrays, objects ({ "cls": bool }), and falsy
 * values — returns a single space-separated class string with Tailwind
 * conflicts resolved via tailwind-merge.
 */
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
