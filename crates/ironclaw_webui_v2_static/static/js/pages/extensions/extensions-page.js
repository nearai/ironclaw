import { Navigate, useParams } from "react-router";
import { React, html } from "../../lib/html.js";
import { ActionToast } from "./components/action-toast.js";
import { ChannelsTab } from "./components/channels-tab.js";
import { ConfigureModal } from "./components/configure-modal.js";
import { McpTab } from "./components/mcp-tab.js";
import { RegistryTab } from "./components/registry-tab.js";
import { useExtensions } from "./hooks/useExtensions.js";

export function ExtensionsPage() {
  const { tab = "registry" } = useParams();
  const [configuring, setConfiguring] = React.useState(null);

  const {
    status,
    channels,
    mcpServers,
    channelRegistry,
    mcpRegistry,
    catalogEntries,
    connectableChannels,
    isLoading,
    isBusy,
    actionResult,
    clearResult,
    install,
    activate,
    remove,
    invalidate,
  } = useExtensions();

  const handleConfigure = React.useCallback((extension) => setConfiguring(extension), []);
  const handleInstall = React.useCallback(
    (payload) => install({ ...payload, onNeedsSetup: handleConfigure }),
    [handleConfigure, install]
  );
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
    return html`
      <div className="flex h-full flex-col overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${[1, 2, 3].map(
              (i) => html`
                <div
                  key=${i}
                  className="flex items-center justify-between border-t border-white/[0.06] py-4 first:border-0"
                >
                  <div>
                    <div className="v2-skeleton h-4 w-40 rounded" />
                    <div className="v2-skeleton mt-2 h-3 w-56 rounded" />
                  </div>
                  <div className="v2-skeleton h-7 w-16 rounded-full" />
                </div>
              `
            )}
          </div>
        </div>
      </div>
    `;
  }

  if (tab === "installed") {
    return html`<${Navigate} to="/extensions/registry" replace />`;
  }

  const tabContent = {
    channels: html`<${ChannelsTab}
      status=${status}
      channels=${channels}
      connectableChannels=${connectableChannels}
      channelRegistry=${channelRegistry}
      onActivate=${activate}
      onConfigure=${handleConfigure}
      onRemove=${remove}
      onInstall=${handleInstall}
      isBusy=${isBusy}
    />`,
    mcp: html`<${McpTab}
      mcpServers=${mcpServers}
      mcpRegistry=${mcpRegistry}
      onActivate=${activate}
      onConfigure=${handleConfigure}
      onRemove=${remove}
      onInstall=${handleInstall}
      isBusy=${isBusy}
    />`,
    registry: html`<${RegistryTab}
      catalogEntries=${catalogEntries}
      onInstall=${handleInstall}
      onActivate=${activate}
      onConfigure=${handleConfigure}
      onRemove=${remove}
      isBusy=${isBusy}
    />`,
  };

  if (!tabContent[tab]) {
    return html`<${Navigate} to="/extensions/registry" replace />`;
  }

  return html`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <${ActionToast} result=${actionResult} onDismiss=${clearResult} />
          ${tabContent[tab]}
        </div>
      </div>

      ${configuring &&
      html`
        <${ConfigureModal}
          extension=${configuring}
          onActivate=${handleActivateFromModal}
          onClose=${handleCloseModal}
          onSaved=${handleSaved}
        />
      `}
    </div>
  `;
}
