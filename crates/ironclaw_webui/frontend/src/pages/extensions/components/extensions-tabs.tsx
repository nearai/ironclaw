import { VerticalTabs, VerticalTabsMobile } from "@ironclaw/ui";
import { useT } from "../../../lib/i18n";
import { EXTENSIONS_TABS } from "../lib/extensions-schema";

function extensionsTabItems(t, counts) {
  return EXTENSIONS_TABS.map((tab) => ({
    id: tab.id,
    label: t(tab.labelKey),
    icon: tab.icon,
    count: counts[tab.id] != null ? counts[tab.id] : undefined,
  }));
}

export function ExtensionsTabs({ activeTab, onTabChange, counts }) {
  const t = useT();
  return (
    <VerticalTabs
      items={extensionsTabItems(t, counts)}
      activeId={activeTab}
      onSelect={onTabChange}
      label={t("extensions.tabs.label")}
    />
  );
}

export function ExtensionsTabsMobile({ activeTab, onTabChange, counts }) {
  const t = useT();
  return (
    <VerticalTabsMobile
      items={extensionsTabItems(t, counts)}
      activeId={activeTab}
      onSelect={onTabChange}
      label={t("extensions.tabs.label")}
    />
  );
}
