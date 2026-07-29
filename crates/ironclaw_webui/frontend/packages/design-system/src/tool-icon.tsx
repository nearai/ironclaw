/**
 * ToolIcon — the chip that identifies a tool, service, or project.
 *
 * Known tools resolve to a glyph from the system icon set (Gmail gets the
 * mail glyph, GitHub its mark, and so on); anything unrecognized falls
 * back to a pixel-face monogram of its first letter, so a new integration
 * never renders blank. Pass `icon` to force a specific glyph. Used as the
 * `leading` slot of ListRow in connection lists, activity feeds, and run
 * steps.
 */
import { cn } from "./cn";
import { Icon } from "./icons";

const TOOL_GLYPHS: Record<string, string> = {
  gmail: "mail",
  email: "mail",
  mail: "mail",
  calendar: "calendar",
  "google calendar": "calendar",
  github: "github",
  slack: "chat",
  terminal: "terminal",
  vault: "lock",
  docs: "file",
  drive: "folder",
};

const CHIP_SIZES = { sm: "h-6 w-6", md: "h-7 w-7", lg: "h-8 w-8" };
const GLYPH_SIZES = { sm: "h-3 w-3", md: "h-3.5 w-3.5", lg: "h-4 w-4" };

export interface ToolIconProps {
  /** Tool/service/project name; drives glyph lookup and the aria label. */
  name: string;
  /** Force a specific glyph from the system icon set. */
  icon?: string;
  size?: keyof typeof CHIP_SIZES;
  shape?: "square" | "circle";
  className?: string;
}

export function ToolIcon({ name, icon, size = "md", shape = "square", className = "" }: ToolIconProps) {
  const glyph = icon ?? TOOL_GLYPHS[name.trim().toLowerCase()];
  return (
    <span
      role="img"
      aria-label={name}
      className={cn(
        "grid shrink-0 place-items-center border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]",
        shape === "circle" ? "rounded-full" : "rounded-[var(--v2-radius-sm)]",
        CHIP_SIZES[size] ?? CHIP_SIZES.md,
        className
      )}
    >
      {glyph ? (
        <Icon name={glyph} className={GLYPH_SIZES[size] ?? GLYPH_SIZES.md} />
      ) : (
        <span className="v2-tag-face text-[0.5625rem] leading-none">
          {(name.trim()[0] ?? "?").toUpperCase()}
        </span>
      )}
    </span>
  );
}
