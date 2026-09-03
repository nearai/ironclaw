import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";
import {
  modelCapabilities,
  type ModelCatalogEntry,
} from "../lib/model-capabilities";

const CAPABILITY_PRESENTATION = {
  text: {
    labelKey: "llm.capabilityText",
    icon: "chat",
    className: "bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]",
  },
  "image-input": {
    labelKey: "llm.capabilityImageInput",
    icon: "eye",
    className: "bg-[var(--v2-info-soft)] text-[var(--v2-info-text)]",
  },
  "image-output": {
    labelKey: "llm.capabilityImageOutput",
    icon: "spark",
    className: "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",
  },
} as const;

type Translate = (key: string, params?: Record<string, unknown>) => string;

export function modelCapabilityDescription(
  entry: ModelCatalogEntry | null | undefined,
  t: Translate,
): string | undefined {
  const labels = modelCapabilities(entry).map((capability) =>
    t(CAPABILITY_PRESENTATION[capability].labelKey)
  );
  return labels.length > 0 ? labels.join(", ") : undefined;
}

export function ModelCapabilityBadges({
  entry,
  className = "",
}: {
  entry?: ModelCatalogEntry | null;
  className?: string;
}) {
  const t = useT();
  const capabilities = modelCapabilities(entry);
  if (capabilities.length === 0) return null;

  return (
    <span
      className={cn("inline-flex min-w-0 flex-wrap items-center gap-1", className)}
      data-testid="model-capability-badges"
    >
      {capabilities.map((capability) => {
        const presentation = CAPABILITY_PRESENTATION[capability];
        const label = t(presentation.labelKey);
        return (
          <span
            key={capability}
            className={cn(
              "inline-flex h-5 w-5 shrink-0 cursor-help items-center justify-center rounded-md transition-colors",
              presentation.className,
            )}
            aria-label={label}
            data-capability={capability}
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
