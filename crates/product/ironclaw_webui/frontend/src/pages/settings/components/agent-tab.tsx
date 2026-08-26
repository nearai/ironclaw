import { Card } from "../../../design-system/card";
import { Skeleton } from "../../../design-system/skeleton";
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
              <Skeleton className="mb-4 h-3 w-20 rounded" />
              {[1, 2, 3, 4].map(
                (j) =>
                  (
                    <div
                      key={j}
                      className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0"
                    >
                      <div>
                        <Skeleton className="h-4 w-32 rounded" />
                        <Skeleton className="mt-1 h-3 w-48 rounded" />
                      </div>
                      <Skeleton className="h-9 w-36 rounded" />
                    </div>
                  )
              )}
            </Card>
          )
      )}
    </div>
  );
}
