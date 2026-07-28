import { Card } from "../../../design-system/card";
import { AGENT_FIELDS } from "../lib/settings-schema";
import { filterSettingsSections } from "../lib/settings-search";
import { SettingsGroup } from "./settings-field";
import { SettingsSearchEmpty } from "./settings-search-empty";
import { useT } from "../../../lib/i18n";

export function AgentTab({
  settings,
  onSave,
  savedKeys,
  isLoading,
  searchQuery = "",
}) {
  const t = useT();
  if (isLoading) {
    return (<AgentSkeleton />);
  }

  const sections = filterSettingsSections(AGENT_FIELDS, settings, searchQuery, t);
  if (sections.length === 0) {
    return (<SettingsSearchEmpty query={searchQuery} />);
  }

  return (
    <div className="space-y-5">
      {sections.map(
        (section) =>
          (
            <SettingsGroup
              key={section.groupKey}
              groupKey={section.groupKey}
              fields={section.fields}
              settings={settings}
              onSave={onSave}
              savedKeys={savedKeys}
            />
          )
      )}
    </div>
  );
}

function AgentSkeleton() {
  return (
    <div className="space-y-5">
      {[1, 2, 3].map(
        (i) =>
          (
            <Card key={i} padding="md">
              <div className="mb-4 h-3 w-20 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              {[1, 2, 3, 4].map(
                (j) =>
                  (
                    <div
                      key={j}
                      className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0"
                    >
                      <div>
                        <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                        <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                      </div>
                      <div className="h-9 w-36 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                    </div>
                  )
              )}
            </Card>
          )
      )}
    </div>
  );
}
