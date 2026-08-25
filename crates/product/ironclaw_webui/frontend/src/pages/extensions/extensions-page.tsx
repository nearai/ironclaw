import { Navigate, useParams } from "react-router";
import React from "react";
import { ConfirmDialog } from "../../design-system/confirm-dialog";
import { InlineNotice } from "../../design-system/inline-notice";
import { Skeleton } from "../../design-system/skeleton";
import { PageScroll, PageStack } from "../../layout/page-shell";
import { useT } from "../../lib/i18n";
import { ChannelsTab } from "./components/channels-tab";
import { ConfigureModal } from "./components/configure-modal";
import { CustomMcpRegistrationModal } from "./components/custom-mcp-registration-modal";
import { ToolsTab } from "./components/tools-tab";
import { RegistryTab } from "./components/registry-tab";
import { configureRequest, useExtensions } from "./hooks/useExtensions";
import { useExtensionSetupLanding } from "./hooks/useSetupLanding";
import type { ConfigureFocusHandler } from "./lib/focus-target";
import type { FocusTarget } from "./lib/focus-target";
import type { FocusTargetResolver } from "./lib/focus-target";
import type { InstallFocusHandler } from "./lib/focus-target";

// The banner text/tone follows the *cause* of the failure, not which tab it is
// shown on: a failed catalog (registry) request is always "Extension catalog
// unavailable" (danger), while a failed installed-extension enrichment request
// is "Some extension data is unavailable" (warning). Whether the banner blocks
// the whole tab or renders inline above still depends on the tab (see below).
function CatalogErrorBanner({ isCatalogError = true, isRefetching, onRetry }) {
  const t = useT();
  const titleKey = isCatalogError
    ? "ext.catalog.loadErrorTitle"
    : "ext.catalog.partialErrorTitle";
  const descriptionKey = isCatalogError
    ? "ext.catalog.loadErrorDesc"
    : "ext.catalog.partialErrorDesc";

  return (
    <InlineNotice
      tone={isCatalogError ? "danger" : "warning"}
      role="alert"
      action={(
        <button
          type="button"
          className="rounded-md border border-current px-3 py-1.5 text-sm font-medium transition-opacity hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-50"
          onClick={onRetry}
          disabled={isRefetching}
        >
          {isRefetching ? t("ext.catalog.retrying") : t("ext.catalog.retry")}
        </button>
      )}
    >
      <p className="text-sm font-semibold">{t(titleKey)}</p>
      <p className="mt-1 text-sm">{t(descriptionKey)}</p>
    </InlineNotice>
  );
}

function ActionNotice({ result, onDismiss }) {
  const t = useT();
  React.useEffect(() => {
    if (!result) return;
    const timer = setTimeout(onDismiss, 4000);
    return () => clearTimeout(timer);
  }, [result, onDismiss]);

  if (!result) return null;

  return (
    <InlineNotice
      tone={result.type === "success" ? "success" : result.type === "error" ? "danger" : "info"}
      role={result.type === "error" ? "alert" : "status"}
      onDismiss={onDismiss}
      dismissLabel={t("common.dismiss")}
    >
      {result.message}
    </InlineNotice>
  );
}

/**
 * @param {string | { id?: string } | null | undefined} packageRef
 * @param {HTMLElement | null | undefined} installTrigger
 * @returns {FocusTargetResolver}
 */
function installReturnFocusTarget(packageRef, installTrigger) {
  const extensionId =
    typeof packageRef === "string" ? packageRef : packageRef?.id;
  return () => {
    if (installTrigger?.isConnected) return installTrigger;
    if (!extensionId) return null;

    const installedCard = Array.from(
      document.querySelectorAll("[data-extension-id]"),
    ).find(
      (card) => card.getAttribute("data-extension-id") === extensionId,
    );
    return /** @type {HTMLElement | null} */ (
      installedCard?.querySelector(
        "[data-extension-return-focus]",
      ) ||
      installedCard?.querySelector(
        "[data-extension-primary-action]",
      ) ||
      (installedCard?.matches("[data-extension-return-focus]")
        ? installedCard
        : null)
    );
  };
}

export function ExtensionsPage({ isAdmin = false } = {}) {
  const t = useT();
  const { tab = "registry" } = useParams();
  const [configuring, setConfiguring] = React.useState(null);
  const [registeringCustomMcp, setRegisteringCustomMcp] = React.useState(false);
  const [extensionToRemove, setExtensionToRemove] = React.useState(null);
  const configureTriggerRef = React.useRef(
    /** @type {FocusTarget | null} */ (null),
  );

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
    registerCustomMcp,
    isRegisteringCustomMcp,
    remove,
    isRemoving,
    importTool,
    isImporting,
    invalidate,
  } = useExtensions();

  /** @type {ConfigureFocusHandler} */
  const handleConfigure = React.useCallback((extension, returnFocusTo) => {
    configureTriggerRef.current = returnFocusTo ||
      /** @type {HTMLElement | null} */ (document.activeElement);
    setConfiguring(extension);
  }, []);
  /** @type {InstallFocusHandler} */
  const handleInstall = React.useCallback(
    (payload, installTrigger) => {
      const returnFocusTo = installReturnFocusTarget(
        payload.packageRef,
        installTrigger,
      );
      install({
        ...payload,
        onNeedsSetup: (extension) =>
          handleConfigure(extension, returnFocusTo),
      });
    },
    [handleConfigure, install]
  );
  // A device-link setup link (`?configure=<id>&setup=personal_account`) opens
  // the same modal the Configure button does. Resolved against the caller's own
  // installed channels and tools, which is where a configurable extension
  // lives — a registry card has nothing to configure yet.
  // Normalized here, not inside the hook: `channels`/`tools` are raw API items
  // (`package_ref`), while everything downstream of Configure expects the
  // `packageRef`/`displayName` shape the card builds. Normalizing at the
  // boundary means the landing resolves and the modal opens on the same object
  // the Configure button would have handed it.
  const configurableExtensions = React.useMemo(
    () => [...(channels || []), ...(tools || [])].map(configureRequest),
    [channels, tools],
  );
  const { setupPath, clearSetupPath } = useExtensionSetupLanding({
    extensions: configurableExtensions,
    isLoading: isExtensionsLoading,
    onConfigure: handleConfigure,
    selected: configuring,
  });
  const handleImport = React.useCallback((file) => importTool({ file }), [importTool]);
  // Closing also releases the deep link's setup path: it applies only to the
  // modal it opened, never to whatever Configure action comes after it.
  const handleCloseModal = React.useCallback(() => {
    setConfiguring(null);
    clearSetupPath();
  }, [clearSetupPath]);
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
      <PageScroll>
        <PageStack>
          {[1, 2, 3].map(
            (i) => (
              <div
                key={i}
                className="flex items-center justify-between border-t border-white/[0.06] py-4 first:border-0"
              >
                <div>
                  <Skeleton className="h-4 w-40 rounded" />
                  <Skeleton className="mt-2 h-3 w-56 rounded" />
                </div>
                <Skeleton className="h-7 w-16 rounded-full" />
              </div>
            )
          )}
        </PageStack>
      </PageScroll>
    );
  }

  const blockingError = tab === "registry" ? registryError : extensionsError;
  if (blockingError) {
    return (
      <PageScroll>
        <CatalogErrorBanner
          isCatalogError={tab === "registry"}
          isRefetching={isRefetching}
          onRetry={refetch}
        />
      </PageScroll>
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
    <PageScroll
      overlay={(
        <>
          {configuring &&
          (
            <ConfigureModal
              extension={configuring}
              initialConnection={setupPath}
              onClose={handleCloseModal}
              onSaved={handleSaved}
              returnFocusTo={configureTriggerRef.current}
            />
          )}
          <CustomMcpRegistrationModal
            open={registeringCustomMcp}
            onClose={() => setRegisteringCustomMcp(false)}
            onRegister={registerCustomMcp}
            isRegistering={isRegisteringCustomMcp}
          />
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
        </>
      )}
    >
      <PageStack>
        <ActionNotice result={actionResult} onDismiss={clearResult} />
        <div className="flex justify-end">
          <button
            type="button"
            className="rounded-md bg-signal px-3 py-2 text-sm font-medium text-white transition-opacity hover:opacity-90"
            onClick={() => setRegisteringCustomMcp(true)}
          >
            {t("extensions.addCustomMcp")}
          </button>
        </div>
        {secondaryError &&
        (<CatalogErrorBanner
            isCatalogError={tab !== "registry"}
            isRefetching={isRefetching}
            onRetry={refetch}
        />)}
        {tabContent[tab]}
      </PageStack>
    </PageScroll>
  );
}
