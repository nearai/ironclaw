import { Navigate, useParams } from "react-router";
import { React, html } from "../../lib/html.js";
import { ActionToast } from "./components/action-toast.js";
import { ChannelsTab } from "./components/channels-tab.js";
import { ConfigureModal } from "./components/configure-modal.js";
import { InstalledTab } from "./components/installed-tab.js";
import { McpTab } from "./components/mcp-tab.js";
import { RegistryTab } from "./components/registry-tab.js";
import { useExtensions } from "./hooks/useExtensions.js";

export function ExtensionsPage() {
  const { tab = "installed" } = useParams();
  const [configuring, setConfiguring] = React.useState(null);

  const {
    status,
    extensions,
    channels,
    mcpServers,
    tools,
    channelRegistry,
    mcpRegistry,
    toolRegistry,
    isLoading,
    isBusy,
    actionResult,
    clearResult,
    install,
    activate,
    remove,
    invalidate,
  } = useExtensions();

  const handleConfigure = React.useCallback((name) => setConfiguring(name), []);
  const handleCloseModal = React.useCallback(() => setConfiguring(null), []);
  const handleSaved = React.useCallback(() => invalidate(), [invalidate]);

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

  const tabContent = {
    installed: html`<${InstalledTab}
      extensions=${extensions}
      onActivate=${activate}
      onConfigure=${handleConfigure}
      onRemove=${remove}
      isBusy=${isBusy}
    />`,
    channels: html`<${ChannelsTab}
      status=${status}
      channels=${channels}
      channelRegistry=${channelRegistry}
      onActivate=${activate}
      onConfigure=${handleConfigure}
      onRemove=${remove}
      onInstall=${install}
      isBusy=${isBusy}
    />`,
    mcp: html`<${McpTab}
      mcpServers=${mcpServers}
      mcpRegistry=${mcpRegistry}
      onActivate=${activate}
      onConfigure=${handleConfigure}
      onRemove=${remove}
      onInstall=${install}
      isBusy=${isBusy}
    />`,
    registry: html`<${RegistryTab}
      toolRegistry=${toolRegistry}
      channelRegistry=${channelRegistry}
      mcpRegistry=${mcpRegistry}
      onInstall=${install}
      isBusy=${isBusy}
    />`,
  };

  if (!tabContent[tab]) {
    return html`<${Navigate} to="/extensions/installed" replace />`;
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
          extensionName=${configuring}
          onClose=${handleCloseModal}
          onSaved=${handleSaved}
        />
      `}
    </div>
  `;
}
