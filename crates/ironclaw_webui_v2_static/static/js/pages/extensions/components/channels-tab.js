import { StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { ExtensionCard, RegistryCard } from "./extension-card.js";
import { PairingSection } from "./pairing-section.js";

export function ChannelsTab({
  status,
  channels,
  channelRegistry,
  onActivate,
  onConfigure,
  onRemove,
  onInstall,
  isBusy,
}) {
  const enabledChannels = status.enabled_channels || [];

  return html`
    <div className="space-y-5">
      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          Built-in channels
        </h3>
        <${BuiltinRow}
          name="Web Gateway"
          description="Browser-based chat with SSE streaming"
          enabled=${true}
          detail=${"SSE: " +
          (status.sse_connections || 0) +
          " · WS: " +
          (status.ws_connections || 0)}
        />
        <${BuiltinRow}
          name="HTTP Webhook"
          description="Inbound webhook endpoint for external integrations"
          enabled=${enabledChannels.includes("http")}
          detail="ENABLE_HTTP=true"
        />
        <${BuiltinRow}
          name="CLI"
          description="Terminal interface with TUI or simple REPL"
          enabled=${enabledChannels.includes("cli")}
          detail="ironclaw run --cli"
        />
        <${BuiltinRow}
          name="REPL"
          description="Minimal read-eval-print loop for testing"
          enabled=${enabledChannels.includes("repl")}
          detail="ironclaw run --repl"
        />
      </div>

      ${channels.length > 0 &&
      html`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            Messaging channels
          </h3>
          ${channels.map(
            (ch) => html`
              <div key=${ch.name}>
                <${ExtensionCard}
                  ext=${ch}
                  onActivate=${onActivate}
                  onConfigure=${onConfigure}
                  onRemove=${onRemove}
                  isBusy=${isBusy}
                />
                ${(ch.onboarding_state === "pairing_required" ||
                  ch.onboarding_state === "pairing") &&
                html` <${PairingSection} channel=${ch.name} /> `}
              </div>
            `
          )}
        </div>
      `}
      ${channelRegistry.length > 0 &&
      html`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            Available channels
          </h3>
          ${channelRegistry.map(
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

function BuiltinRow({ name, description, enabled, detail }) {
  return html`
    <div
      className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-iron-200">${name}</span>
          <${StatusPill}
            tone=${enabled ? "success" : "muted"}
            label=${enabled ? "on" : "off"}
          />
        </div>
        <div className="mt-1 text-xs text-iron-300">${description}</div>
        ${detail &&
        html`<div className="mt-1 font-mono text-[11px] text-iron-700">
          ${detail}
        </div>`}
      </div>
    </div>
  `;
}
