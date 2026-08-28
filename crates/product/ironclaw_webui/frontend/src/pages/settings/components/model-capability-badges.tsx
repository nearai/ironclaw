import { Badge } from "../../../design-system/badge";
import { cn } from "../../../utils/cn";
import {
  capabilityLabels,
  type ModelCatalogEntry,
} from "../lib/model-capabilities";

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
      {labels.map((label) => (
        <Badge key={label} dot={false} label={label} size="sm" tone="muted" />
      ))}
    </span>
  );
}
