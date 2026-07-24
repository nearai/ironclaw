import { Navigate, useParams } from "react-router";
import React from "react";
import { ConfirmDialog } from "../../design-system/confirm-dialog";
import { useT } from "../../lib/i18n";
import { ActionToast } from "./components/action-toast";
import { ChannelsTab } from "./components/channels-tab";
import { ConfigureModal } from "./components/configure-modal";
import { ToolsTab } from "./components/tools-tab";
import { RegistryTab } from "./components/registry-tab";
import { useExtensions } from "./hooks/useExtensions";

// The banner text/tone follows the *cause* of the failure, not which tab it is
// shown on: a failed catalog (registry) request is always "Extension catalog
// unavailable" (danger), while a failed installed-extension enrichment request
// is "Some extension data is unavailable" (warning). Whether the banner blocks
// the whole tab or renders inline above still depends on the tab (see below).
function CatalogErrorBanner({ isCatalogError = true, isRefetching, onRetry }) {
  const t = useT();
  const toneClass = isCatalogError
    ? "border-[color-mix(in_srgb,var(--v2-danger-text)_30%,transparent)] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]"
    : "border-[color-mix(in_srgb,var(--v2-warning-text)_30%,transparent)] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]";
  const titleKey = isCatalogError
    ? "ext.catalog.loadErrorTitle"
    : "ext.catalog.partialErrorTitle";
  const descriptionKey = isCatalogError
    ? "ext.catalog.loadErrorDesc"
    : "ext.catalog.partialErrorDesc";

  return (
    <div
      className={`rounded-lg border px-4 py-4 ${toneClass}`}
      role="alert"
    >
      <p className="text-sm font-semibold">{t(titleKey)}</p>
      <p className="mt-1 text-sm">{t(descriptionKey)}</p>
      <button
        type="button"
        className="mt-4 rounded-md border border-current px-3 py-1.5 text-sm font-medium transition-opacity hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-50"
        onClick={onRetry}
        disabled={isRefetching}
      >
        {isRefetching ? t("ext.catalog.retrying") : t("ext.catalog.retry")}
      </button>
    </div>
  );
}

export function ExtensionsPage({ isAdmin = false } = {}) {
  const t = useT();
  const { tab = "registry" } = useParams();
  const [configuring, setConfiguring] = React.useState(null);
  const [extensionToRemove, setExtensionToRemove] = React.useState(null);

  const {
    status,
    channels,
    tools,
    channelRegistry,
    toolRegistry,
    catalogEntries,
    isExtensionsLoading,
    isRegistryLoading,
    extensionsError,
    registryError,
    refetch,
    isRefetching,
    isBusy,
    actionResult,
    clearResult,
    install,
    remove,
    isRemoving,
    importTool,
    isImporting,
    invalidate,
  } = useExtensions();

  const handleConfigure = React.useCallback((extension) => setConfiguring(extension), []);
  const handleInstall = React.useCallback(
    (payload) => install({ ...payload, onNeedsSetup: handleConfigure }),
    [handleConfigure, install]
  );
  const handleImport = React.useCallback((file) => importTool({ file }), [importTool]);
  const handleCloseModal = React.useCallback(() => setConfiguring(null), []);
  const handleConfirmRemove = React.useCallback(() => {
    if (!extensionToRemove) return;
    remove(extensionToRemove, {
      onSettled: () => setExtensionToRemove(null),
    });
  }, [extensionToRemove, remove]);
  const handleSaved = React.useCallback(() => invalidate(), [invalidate]);
  // `mcp` was the pre-unification name of the tools view; keep main-era deep
  // links working while the canonical tab id is `tools` (product taxonomy —
  // MCP is a runtime badge, never a grouping axis).
  if (tab === "mcp") {
    return (<Navigate to="/extensions/tools" replace />);
  }
  if (!["channels", "tools", "registry"].includes(tab)) {
    return (<Navigate to="/extensions/registry" replace />);
  }

  // The registry response already contains every catalog entry plus its
  // installed flag. Render that snapshot as soon as it arrives; the slower
  // installed-extension request can progressively replace installed registry
  // cards with their full management controls when enrichment finishes.
  const isLoading = isRegistryLoading || (tab !== "registry" && isExtensionsLoading);

  if (isLoading) {
    return (
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            {[1, 2, 3].map(
              (i) => (
                <div
                  key={i}
                  className="flex items-center justify-between border-t border-white/[0.06] py-4 first:border-0"
                >
                  <div>
                    <div className="v2-skeleton h-4 w-40 rounded" />
                    <div className="v2-skeleton mt-2 h-3 w-56 rounded" />
                  </div>
                  <div className="v2-skeleton h-7 w-16 rounded-full" />
                </div>
              )
            )}
          </div>
        </div>
      </div>
    );
  }

  const blockingError = tab === "registry" ? registryError : extensionsError;
  if (blockingError) {
    return (
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <CatalogErrorBanner
            isCatalogError={tab === "registry"}
            isRefetching={isRefetching}
            onRetry={refetch}
          />
        </div>
      </div>
    );
  }

  const tabContent = {
    channels: (<ChannelsTab
      channels={channels}
      channelRegistry={channelRegistry}
      onConfigure={handleConfigure}
      onRemove={setExtensionToRemove}
      onInstall={handleInstall}
      isBusy={isBusy}
    />),
    tools: (<ToolsTab
      tools={tools}
      toolRegistry={toolRegistry}
      onConfigure={handleConfigure}
      onRemove={setExtensionToRemove}
      onInstall={handleInstall}
      isBusy={isBusy}
    />),
    registry: (<RegistryTab
      catalogEntries={catalogEntries}
      onInstall={handleInstall}
      onConfigure={handleConfigure}
      onRemove={setExtensionToRemove}
      onImport={handleImport}
      isAdmin={isAdmin}
      isImporting={isImporting}
      isBusy={isBusy}
    />),
  };

  // The secondary (non-primary) query for this tab. On the registry tab that is
  // the installed-extension enrichment; on the channels/mcp tabs it is the
  // catalog. Render it inline above the tab content, with cause-driven text.
  const secondaryError = tab === "registry" ? extensionsError : registryError;

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <ActionToast result={actionResult} onDismiss={clearResult} />
          {secondaryError &&
          (<CatalogErrorBanner
            isCatalogError={tab !== "registry"}
            isRefetching={isRefetching}
            onRetry={refetch}
          />)}
          {tabContent[tab]}
        </div>
      </div>

      {configuring &&
      (
        <ConfigureModal
          extension={configuring}
          onClose={handleCloseModal}
          onSaved={handleSaved}
        />
      )}
      <ConfirmDialog
        open={Boolean(extensionToRemove)}
        title={`${t("common.remove")}: ${
          extensionToRemove?.displayName ||
          extensionToRemove?.packageRef?.id ||
          t("extensions.defaultName")
        }`}
        confirmLabel={t("common.remove")}
        isConfirming={isRemoving}
        onConfirm={handleConfirmRemove}
        onCancel={() => setExtensionToRemove(null)}
      />
    </div>
  );
}
