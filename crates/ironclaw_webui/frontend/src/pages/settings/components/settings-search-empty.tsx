import { Card } from "../../../design-system/card";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";

export function SettingsSearchEmpty({ query }) {
  const t = useT();

  return (
    <Card padding="lg">
      <div className="flex items-center gap-3">
        <span
          className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-faint)]"
        >
          <Icon name="search" className="h-4 w-4" />
        </span>
        <div className="min-w-0">
          <h3 className="text-sm font-semibold text-[var(--v2-text-strong)]">
            {t("settings.noMatchingSettings", { query })}
          </h3>
        </div>
      </div>
    </Card>
  );
}
