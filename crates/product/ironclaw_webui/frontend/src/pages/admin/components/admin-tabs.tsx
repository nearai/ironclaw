import { Icon } from "../../../design-system/icons";
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
                ? "v2-nav-active text-white"
                : "text-iron-300 hover:bg-white/[0.045] hover:text-white",
            ].join(" ")}
          >
            <span
              className={[
                "grid h-7 w-7 shrink-0 place-items-center rounded-md border",
                activeTab === tab.id
                  ? "border-signal/35 bg-signal/10 text-signal"
                  : "border-white/10 bg-white/[0.035] text-iron-300 group-hover:border-signal/35 group-hover:text-signal",
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
                ? "border border-signal/35 bg-signal/10 text-white"
                : "border border-transparent text-iron-300 hover:text-white",
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
