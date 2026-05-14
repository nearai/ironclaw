import { html } from "../../../lib/html.js";
import { ExtensionCard, RegistryCard } from "./extension-card.js";

export function McpTab({
  mcpServers,
  mcpRegistry,
  onActivate,
  onConfigure,
  onRemove,
  onInstall,
  isBusy,
}) {
  if (mcpServers.length === 0 && mcpRegistry.length === 0) {
    return html`
      <div className="v2-panel rounded-[18px] p-6 sm:p-8">
        <h3 className="text-lg font-semibold text-white">No MCP servers</h3>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          MCP servers extend the agent with additional tool capabilities over
          the Model Context Protocol. Install them from the registry.
        </p>
      </div>
    `;
  }

  return html`
    <div className="space-y-5">
      ${mcpServers.length > 0 &&
      html`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            Installed MCP servers
          </h3>
          ${mcpServers.map(
            (ext) => html`
              <${ExtensionCard}
                key=${ext.name}
                ext=${ext}
                onActivate=${onActivate}
                onConfigure=${onConfigure}
                onRemove=${onRemove}
                isBusy=${isBusy}
              />
            `
          )}
        </div>
      `}
      ${mcpRegistry.length > 0 &&
      html`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            Available MCP servers
          </h3>
          ${mcpRegistry.map(
            (entry) => html`
              <${RegistryCard}
                key=${entry.name}
                entry=${entry}
                onInstall=${onInstall}
                isBusy=${isBusy}
              />
            `
          )}
        </div>
      `}
    </div>
  `;
}
