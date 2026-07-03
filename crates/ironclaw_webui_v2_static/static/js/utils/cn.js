/**
 * Tiny class-name merger.
 *
 * Accepts any mix of strings, arrays, objects ({ "cls": bool }), and falsy
 * values — returns a single space-separated class string.
 *
 * Usage:
 *   cn("base", isActive && "active", ["also", "works"])
 *   cn("base", condition ? "a" : "b", { "font-bold": isBold }, className)
 */
export function cn(...inputs) {
  const result = [];
  for (const input of inputs) {
    if (!input) continue;
    if (typeof input === "string") {
      result.push(input);
    } else if (Array.isArray(input)) {
      const inner = cn(...input);
      if (inner) result.push(inner);
    } else if (typeof input === "object") {
      for (const [key, val] of Object.entries(input)) {
        if (val) result.push(key);
      }
    }
  }
  return result.join(" ");
}
