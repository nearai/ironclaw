import { Icon } from "@ironclaw/design-system";
import { useT } from "../../../lib/i18n";

const ADMIN_TABS = [
  { id: "dashboard", labelKey: "admin.tab.dashboard", icon: "pulse" },
  { id: "users", labelKey: "admin.tab.users", icon: "lock" },
  { id: "usage", labelKey: "admin.tab.usage", icon: "spark" },
];

export { ADMIN_TABS };

export function AdminTabs({ activeTab, onTabChange }) {
  const t = useT();
  return (
    <div className="flex flex-col gap-1">
      {ADMIN_TABS.map(
        (tab) => (
          <button
            key={tab.id}
            onClick={() => onTabChange(tab.id)}
            className={[
              "group flex items-center gap-3 rounded-md px-3 py-2.5 text-left text-sm",
              activeTab === tab.id
                ? "v2-nav-active text-[var(--v2-text-strong)]"
                : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
            ].join(" ")}
          >
            <span
              className={[
                "grid h-7 w-7 shrink-0 place-items-center rounded-md border",
                activeTab === tab.id
                  ? "border-[color-mix(in_srgb,var(--v2-accent)_35%,transparent)] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                  : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] group-hover:border-[color-mix(in_srgb,var(--v2-accent)_35%,transparent)] group-hover:text-[var(--v2-accent-text)]",
              ].join(" ")}
            >
              <Icon name={tab.icon} className="h-3.5 w-3.5" />
            </span>
            <span className="min-w-0 truncate">{t(tab.labelKey)}</span>
          </button>
        )
      )}
    </div>
  );
}

export function AdminTabsMobile({ activeTab, onTabChange }) {
  const t = useT();
  return (
    <div className="flex gap-1.5 overflow-x-auto pb-1">
      {ADMIN_TABS.map(
        (tab) => (
          <button
            key={tab.id}
            onClick={() => onTabChange(tab.id)}
            className={[
              "flex shrink-0 items-center gap-2 rounded-md px-3 py-2 text-sm whitespace-nowrap",
              activeTab === tab.id
                ? "border border-[color-mix(in_srgb,var(--v2-accent)_35%,transparent)] bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
                : "border border-transparent text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]",
            ].join(" ")}
          >
            <Icon name={tab.icon} className="h-3.5 w-3.5" />
            {t(tab.labelKey)}
          </button>
        )
      )}
    </div>
  );
}
