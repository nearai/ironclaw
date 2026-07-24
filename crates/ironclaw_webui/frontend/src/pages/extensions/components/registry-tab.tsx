import React from "react";
import { useT } from "../../../lib/i18n";
import { Icon } from "../../../design-system/icons";
import { ExtensionCard, RegistryCard } from "./extension-card";

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
      <button
        type="button"
        onClick={() => fileInputRef.current?.click()}
        disabled={isBusy || isImporting}
        className="flex items-center gap-1.5 rounded-md border border-white/12 bg-white/[0.04] px-2.5 py-1 text-xs text-iron-100 transition hover:bg-white/[0.08] disabled:opacity-50"
      >
        <Icon name="upload" className="h-3 w-3" />
        {isImporting ? t("ext.registry.importing") : t("ext.registry.import")}
      </button>
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
      <div className="v2-panel rounded-[18px] p-6 sm:p-8">
        <div className="flex items-start justify-between gap-4">
          <h3 className="text-lg font-semibold text-white">
            {t("ext.registry.emptyTitle")}
          </h3>
          {importControl}
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          {t("ext.registry.emptyDesc")}
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <input
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.currentTarget.value)}
          placeholder={t("ext.registry.searchPlaceholder")}
          className="h-9 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <span className="font-mono text-[11px] text-iron-700">
          {filtered.length} / {catalogEntries.length}
        </span>
      </div>

      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        {installedCount > 0 &&
        (
          <>
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            {t("extensions.installed")}
          </h3>
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
            <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
              {t("ext.registry.availableTitle")}
            </h3>
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
        (<p className="py-4 text-sm text-iron-300">
          {t("ext.registry.noMatch")}
        </p>)}
      </div>
    </div>
  );
}
