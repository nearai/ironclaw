import { VerticalTabs, VerticalTabsMobile } from "@ironclaw/ui";
import React from "react";
import { useT } from "../../../lib/i18n";
import { SETTINGS_TABS } from "../lib/settings-schema";

function useVisibleTabs(isAdmin) {
  return React.useMemo(
    () =>
      SETTINGS_TABS.filter(
        (tab) => isAdmin || (tab.id !== "users" && tab.id !== "inference")
      ),
    [isAdmin]
  );
}

function useTabItems(isAdmin) {
  const t = useT();
  const tabs = useVisibleTabs(isAdmin);
  return tabs.map((tab) => ({
    id: tab.id,
    label: t(tab.labelKey),
    icon: tab.icon,
  }));
}

export function SettingsTabs({ activeTab, onTabChange, isAdmin = false }) {
  const t = useT();
  const items = useTabItems(isAdmin);
  return (
    <VerticalTabs
      items={items}
      activeId={activeTab}
      onSelect={onTabChange}
      label={t("settings.tabsLabel")}
    />
  );
}

export function SettingsTabsMobile({ activeTab, onTabChange, isAdmin = false }) {
  const t = useT();
  const items = useTabItems(isAdmin);
  return (
    <VerticalTabsMobile
      items={items}
      activeId={activeTab}
      onSelect={onTabChange}
      label={t("settings.tabsLabel")}
    />
  );
}
