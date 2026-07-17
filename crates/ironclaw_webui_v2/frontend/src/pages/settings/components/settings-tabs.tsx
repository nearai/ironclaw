import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { SelectMenu } from "../../../design-system/select-menu";
import React from "react";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";
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

export function SettingsTabs({ activeTab, onTabChange, isAdmin = false }) {
  const t = useT();
  const tabs = useVisibleTabs(isAdmin);
  return (
    <div className="flex flex-col gap-1" role="tablist" aria-label={t("nav.settings")}>
      {tabs.map((tab) => {
        const selected = activeTab === tab.id;
        return (
          <Button
            key={tab.id}
            type="button"
            variant="ghost"
            role="tab"
            aria-selected={selected}
            onClick={() => onTabChange(tab.id)}
            className={cn(
              "h-auto w-full justify-start gap-3 rounded-md px-3 py-2.5 text-left text-sm",
              selected
                ? "bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
                : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)]"
            )}
          >
            <span
              className={cn(
                "grid h-7 w-7 shrink-0 place-items-center rounded-md border",
                selected
                  ? "border-[color-mix(in_srgb,var(--v2-accent)_35%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                  : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"
              )}
            >
              <Icon name={tab.icon} className="h-3.5 w-3.5" />
            </span>
            <span className="min-w-0 truncate">{t(tab.labelKey)}</span>
          </Button>
        );
      })}
    </div>
  );
}

export function SettingsTabsMobile({ activeTab, onTabChange, isAdmin = false }) {
  const t = useT();
  const tabs = useVisibleTabs(isAdmin);
  return (
    <SelectMenu
      ariaLabel={t("nav.settings")}
      value={activeTab}
      onChange={onTabChange}
      align="left"
      className="w-full"
      options={tabs.map((tab) => ({
        value: tab.id,
        label: t(tab.labelKey),
      }))}
    />
  );
}
