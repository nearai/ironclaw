/**
 * Provider-neutral icons for OOBE suggestion cards.
 *
 * The backend selects a semantic task category. Human-readable provenance is
 * carried separately in `sources`; it is never used to derive the icon.
 * Unknown or legacy values degrade to `generic` so persisted suggestions
 * remain renderable when the vocabulary changes.
 */
import { Icon } from "../../../design-system/icons";

export const SUGGESTION_ICON_IDS = [
  "email",
  "calendar",
  "document",
  "storage",
  "spreadsheet",
  "presentation",
  "code",
  "messaging",
  "notes",
  "web",
  "memory",
  "generic",
] as const;

export type SuggestionIconId = (typeof SUGGESTION_ICON_IDS)[number];

const ICON_ID_SET = new Set<string>(SUGGESTION_ICON_IDS);

const GLYPH_NAMES: Record<SuggestionIconId, string> = {
  email: "send",
  calendar: "calendar",
  document: "file",
  storage: "folder",
  spreadsheet: "list",
  presentation: "layers",
  code: "code",
  messaging: "chat",
  notes: "edit",
  web: "search",
  memory: "bookOpen",
  generic: "spark",
};

/** Resolve a schema value, falling back for unknown, missing, or legacy data. */
export function resolveIconId(
  suggestion: { icon?: string | null } | null | undefined,
): SuggestionIconId {
  const icon = suggestion?.icon;
  return icon && ICON_ID_SET.has(icon) ? (icon as SuggestionIconId) : "generic";
}

export function SuggestionIcon({
  id,
  className,
}: {
  id: SuggestionIconId;
  className?: string;
}) {
  return <Icon name={GLYPH_NAMES[id] ?? GLYPH_NAMES.generic} className={className} />;
}
