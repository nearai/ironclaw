import { Navigate, useParams } from "react-router";
import React from "react";
import { ActionToast } from "./components/action-toast";
import { ChannelsTab } from "./components/channels-tab";
import { ConfigureModal } from "./components/configure-modal";
import { McpTab } from "./components/mcp-tab";
import { RegistryTab } from "./components/registry-tab";
import { useExtensions } from "./hooks/useExtensions";

export function ExtensionsPage({ isAdmin = false } = {}) {
  const { tab = "registry" } = useParams();
  const [configuring, setConfiguring] = React.useState(null);

  const {
    status,
    channels,
    mcpServers,
    channelRegistry,
    mcpRegistry,
    catalogEntries,
    isLoading,
    isBusy,
    actionResult,
    clearResult,
    install,
    activate,
    remove,
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
  const handleSaved = React.useCallback(() => invalidate(), [invalidate]);
  const handleActivateFromModal = React.useCallback(
    (extension) => {
      if (!extension) return;
      activate(extension);
      setConfiguring(null);
    },
    [activate]
  );

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

  if (tab === "installed") {
    return (<Navigate to="/extensions/registry" replace />);
  }

  const tabContent = {
    channels: (<ChannelsTab
      status={status}
      channels={channels}
      channelRegistry={channelRegistry}
      onActivate={activate}
      onConfigure={handleConfigure}
      onRemove={remove}
      onInstall={handleInstall}
      isBusy={isBusy}
    />),
    mcp: (<McpTab
      mcpServers={mcpServers}
      mcpRegistry={mcpRegistry}
      onActivate={activate}
      onConfigure={handleConfigure}
      onRemove={remove}
      onInstall={handleInstall}
      isBusy={isBusy}
    />),
    registry: (<RegistryTab
      catalogEntries={catalogEntries}
      onInstall={handleInstall}
      onActivate={activate}
      onConfigure={handleConfigure}
      onRemove={remove}
      onImport={handleImport}
      isAdmin={isAdmin}
      isImporting={isImporting}
      isBusy={isBusy}
    />),
  };

  if (!tabContent[tab]) {
    return (<Navigate to="/extensions/registry" replace />);
  }

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <ActionToast result={actionResult} onDismiss={clearResult} />
          {tabContent[tab]}
        </div>
      </div>

      {configuring &&
      (
        <ConfigureModal
          extension={configuring}
          onActivate={handleActivateFromModal}
          onClose={handleCloseModal}
          onSaved={handleSaved}
        />
      )}
    </div>
  );
}
