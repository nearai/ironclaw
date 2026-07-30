import { VerticalTabs, VerticalTabsMobile, type IconName } from "@ironclaw/ui";
import { useT } from "../../../lib/i18n";

const ADMIN_TABS: { id: string; labelKey: string; icon: IconName }[] = [
  { id: "dashboard", labelKey: "admin.tab.dashboard", icon: "pulse" },
  { id: "users", labelKey: "admin.tab.users", icon: "lock" },
  { id: "usage", labelKey: "admin.tab.usage", icon: "spark" },
];

export { ADMIN_TABS };

function adminTabItems(t) {
  return ADMIN_TABS.map((tab) => ({
    id: tab.id,
    label: t(tab.labelKey),
    icon: tab.icon,
  }));
}

export function AdminTabs({ activeTab, onTabChange }) {
  const t = useT();
  return (
    <VerticalTabs
      items={adminTabItems(t)}
      activeId={activeTab}
      onSelect={onTabChange}
      label={t("admin.tabs.label")}
    />
  );
}

export function AdminTabsMobile({ activeTab, onTabChange }) {
  const t = useT();
  return (
    <VerticalTabsMobile
      items={adminTabItems(t)}
      activeId={activeTab}
      onSelect={onTabChange}
      label={t("admin.tabs.label")}
    />
  );
}
