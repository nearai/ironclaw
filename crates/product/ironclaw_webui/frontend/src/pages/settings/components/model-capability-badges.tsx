import { Icon } from "../../../design-system/icons";
import { cn } from "../../../utils/cn";
import {
  capabilityLabels,
  type ModelCatalogEntry,
} from "../lib/model-capabilities";

const CAPABILITY_PRESENTATION = {
  Text: {
    id: "text",
    icon: "chat",
    className: "bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]",
  },
  "Image input": {
    id: "image-input",
    icon: "eye",
    className: "bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",
  },
  "Image output": {
    id: "image-output",
    icon: "spark",
    className: "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",
  },
} as const;

export function ModelCapabilityBadges({
  entry,
  className = "",
}: {
  entry?: ModelCatalogEntry | null;
  className?: string;
}) {
  const labels = capabilityLabels(entry);
  if (labels.length === 0) return null;

  return (
    <span
      className={cn("inline-flex min-w-0 flex-wrap items-center gap-1", className)}
      data-testid="model-capability-badges"
    >
      {labels.map((label) => {
        const presentation = CAPABILITY_PRESENTATION[label];
        return (
          <span
            key={label}
            className={cn(
              "inline-flex h-5 w-5 shrink-0 cursor-help items-center justify-center rounded-md transition-colors",
              presentation.className,
            )}
            aria-label={label}
            data-capability={presentation.id}
            role="img"
            title={label}
          >
            <Icon
              name={presentation.icon}
              className="h-3.5 w-3.5 shrink-0"
              strokeWidth={1.9}
            />
          </span>
        );
      })}
    </span>
  );
}
