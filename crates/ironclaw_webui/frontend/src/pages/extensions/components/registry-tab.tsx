import React from "react";
import { useT } from "../../../lib/i18n";
import { Button, Icon, Input, Text } from "@ironclaw/design-system";
import { ExtensionCard, RegistryCard } from "./extension-card";
import type {
  ConfigureFocusHandler,
  InstallFocusHandler,
} from "../lib/focus-target";

function packageId(item) {
  return item?.package_ref?.id || "";
}

function catalogItem(entry) {
  return entry.entry || entry.extension || {};
}

function ImportButton({ onImport, isImporting, isBusy }) {
  const t = useT();
  const fileInputRef = React.useRef(null);

  const handleFileChange = React.useCallback(
    (e) => {
      const file = e.target.files?.[0];
      e.target.value = "";
      if (!file || !onImport) return;
      onImport(file);
    },
    [onImport]
  );

  return (
    <div>
      <Button
        type="button"
        variant="secondary"
        size="sm"
        onClick={() => fileInputRef.current?.click()}
        disabled={isBusy || isImporting}
        className="gap-1.5"
      >
        <Icon name="upload" className="h-3 w-3" />
        {isImporting ? t("ext.registry.importing") : t("ext.registry.import")}
      </Button>
      <input
        ref={fileInputRef}
        type="file"
        accept=".zip,application/zip"
        className="hidden"
        onChange={handleFileChange}
      />
    </div>
  );
}

/**
 * @param {{
 *   catalogEntries: any[];
 *   onInstall: InstallFocusHandler;
 *   onConfigure: ConfigureFocusHandler<any>;
 *   onRemove: (extension: any) => void;
 *   onImport: (file: File) => void;
 *   isAdmin: boolean;
 *   isImporting: boolean;
 *   isBusy: boolean;
 * }} props
 */
export function RegistryTab({
  catalogEntries,
  onInstall,
  onConfigure,
  onRemove,
  onImport,
  isAdmin,
  isImporting,
  isBusy,
}) {
  const t = useT();
  const [filter, setFilter] = React.useState("");
  const query = filter.trim().toLowerCase();

  const importControl = isAdmin
    ? (<ImportButton
        onImport={onImport}
        isImporting={isImporting}
        isBusy={isBusy}
      />)
    : null;

  const filtered = query
    ? catalogEntries.filter((entry) => {
        const item = catalogItem(entry);
        return (
          (item.display_name || packageId(item)).toLowerCase().includes(query) ||
          (item.description || "").toLowerCase().includes(query) ||
          (item.keywords || []).some((kw) =>
            kw.toLowerCase().includes(query)
          )
        );
      })
    : catalogEntries;

  const installedEntries = filtered.filter((entry) => entry.installed && entry.extension);
  const registryOnlyInstalledEntries = filtered.filter(
    (entry) => entry.installed && !entry.extension && entry.entry
  );
  const installedCount = installedEntries.length + registryOnlyInstalledEntries.length;
  const availableEntries = filtered.filter((entry) => !entry.installed && entry.entry);

  if (catalogEntries.length === 0) {
    return (
      <div className="v2-panel rounded-[var(--v2-radius-bubble)] p-6 sm:p-8">
        <div className="flex items-start justify-between gap-4">
          <h3 className="text-lg font-medium text-[var(--v2-text-strong)]">
            {t("ext.registry.emptyTitle")}
          </h3>
          {importControl}
        </div>
        <Text variant="body" tone="muted" className="mt-2 max-w-md">
          {t("ext.registry.emptyDesc")}
        </Text>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <Input
          size="lg"
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.currentTarget.value)}
          placeholder={t("ext.registry.searchPlaceholder")}
          className="flex-1"
        />
        <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">
          {filtered.length} / {catalogEntries.length}
        </span>
      </div>

      <div className="v2-panel rounded-[var(--v2-radius-bubble)] p-5 sm:p-6">
        {installedCount > 0 &&
        (
          <>
          <Text as="h3" variant="eyebrow" tone="accent" className="mb-4">
            {t("extensions.installed")}
          </Text>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            {installedEntries.map(
              (entry) => (
                <ExtensionCard
                  key={entry.id}
                  ext={entry.extension || entry.entry}
                  onConfigure={onConfigure}
                  onRemove={onRemove}
                  isBusy={isBusy}
                />
              )
            )}
            {registryOnlyInstalledEntries.map(
              (entry) => (
                <RegistryCard
                  key={entry.id}
                  entry={entry.entry}
                  statusLabel={t("extensions.installed")}
                  isBusy={isBusy}
                />
              )
            )}
          </div>
          </>
        )}

        {(availableEntries.length > 0 || isAdmin) &&
        (
          <>
          <div
            className={[
              "mb-4 flex items-center justify-between",
              installedCount > 0 ? "mt-6" : "",
            ].join(" ")}
          >
            <Text as="h3" variant="eyebrow" tone="accent">
              {t("ext.registry.availableTitle")}
            </Text>
            {importControl}
          </div>
          {availableEntries.length > 0 &&
          (
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
              {availableEntries.map(
                (entry) => (
                  <RegistryCard
                    key={entry.id}
                    entry={entry.entry}
                    onInstall={onInstall}
                    isBusy={isBusy}
                  />
                )
              )}
            </div>
          )}
          </>
        )}

        {filtered.length === 0 &&
        (<Text variant="body" tone="muted" className="py-4">
          {t("ext.registry.noMatch")}
        </Text>)}
      </div>
    </div>
  );
}
