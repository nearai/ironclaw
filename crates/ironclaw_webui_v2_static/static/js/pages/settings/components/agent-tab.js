import { html } from "../../../lib/html.js";
import { Card } from "../../../design-system/card.js";
import { AGENT_FIELDS } from "../lib/settings-schema.js";
import { filterSettingsSections } from "../lib/settings-search.js";
import { SettingsGroup } from "./settings-field.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";
import { useT } from "../../../lib/i18n.js";

export function AgentTab({
  settings,
  onSave,
  savedKeys,
  isLoading,
  searchQuery = "",
}) {
  const t = useT();
  if (isLoading) {
    return html`<${AgentSkeleton} />`;
  }

  const sections = filterSettingsSections(AGENT_FIELDS, settings, searchQuery, t);
  if (sections.length === 0) {
    return html`<${SettingsSearchEmpty} query=${searchQuery} />`;
  }

  return html`
    <div className="space-y-5">
      ${sections.map(
        (section) =>
          html`
            <${SettingsGroup}
              key=${section.groupKey}
              groupKey=${section.groupKey}
              fields=${section.fields}
              settings=${settings}
              onSave=${onSave}
              savedKeys=${savedKeys}
            />
          `
      )}
    </div>
  `;
}

function AgentSkeleton() {
  return html`
    <div className="space-y-5">
      ${[1, 2, 3].map(
        (i) =>
          html`
            <${Card} key=${i} padding="md">
              <div className="mb-4 h-3 w-20 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              ${[1, 2, 3, 4].map(
                (j) =>
                  html`
                    <div
                      key=${j}
                      className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0"
                    >
                      <div>
                        <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                        <div className="mt-1 h-3 w-48 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                      </div>
                      <div className="h-9 w-36 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                    </div>
                  `
              )}
            <//>
          `
      )}
    </div>
  `;
}
