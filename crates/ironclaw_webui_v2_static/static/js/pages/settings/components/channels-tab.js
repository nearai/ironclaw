import { Badge } from "../../../design-system/badge.js";
import { Card } from "../../../design-system/card.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { useChannels } from "../hooks/useChannels.js";
import { matchesSearch } from "../lib/settings-search.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";

function BuiltinChannelCard({ name, description, enabled, detail }) {
  const t = useT();
  return html`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${name}</span>
          <${Badge}
            tone=${enabled ? "positive" : "muted"}
            label=${enabled ? t("channels.statusOn") : t("channels.statusOff")}
            size="sm"
          />
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${description}</div>
        ${detail &&
        html`<div className="mt-1 font-mono text-[11px] text-[var(--v2-text-faint)]">
          ${detail}
        </div>`}
      </div>
    </div>
  `;
}

function ExtensionChannelCard({ channel, registryEntry }) {
  const t = useT();
  const name =
    registryEntry?.display_name ||
    channel?.name ||
    registryEntry?.name ||
    t("common.unknown");
  const desc = registryEntry?.description || channel?.description || "";
  const isInstalled = Boolean(channel);
  const state = channel?.onboarding_state || "setup_required";

  const toneMap = {
    ready: "positive",
    auth_required: "warning",
    pairing_required: "warning",
    setup_required: "muted",
  };
  const labelMap = {
    ready: t("channels.ready"),
    auth_required: t("channels.authNeeded"),
    pairing_required: t("channels.pairing"),
    setup_required: t("channels.setup"),
  };

  return html`
    <div
      className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]">${name}</span>
          ${isInstalled
            ? html`<${Badge}
                tone=${toneMap[state] || "muted"}
                label=${labelMap[state] || state}
                size="sm"
              />`
            : html`<${Badge}
                tone="muted"
                label=${t("channels.available")}
                size="sm"
              />`}
        </div>
        <div className="mt-1 text-xs text-[var(--v2-text-muted)]">${desc}</div>
      </div>
    </div>
  `;
}

function buildBuiltInChannels(status, t) {
  const enabledChannels = status.enabled_channels || [];
  return [
    {
      id: "web",
      name: t("channels.webGateway"),
      description: t("channels.webGatewayDesc"),
      enabled: true,
      detail:
        "SSE: " +
        (status.sse_connections || 0) +
        " · WS: " +
        (status.ws_connections || 0),
    },
    {
      id: "http",
      name: t("channels.httpWebhook"),
      description: t("channels.httpWebhookDesc"),
      enabled: enabledChannels.includes("http"),
      detail: "ENABLE_HTTP=true",
    },
    {
      id: "cli",
      name: t("channels.cli"),
      description: t("channels.cliDesc"),
      enabled: enabledChannels.includes("cli"),
      detail: "ironclaw run --cli",
    },
    {
      id: "repl",
      name: t("channels.repl"),
      description: t("channels.replDesc"),
      enabled: enabledChannels.includes("repl"),
      detail: "ironclaw run --repl",
    },
  ];
}

function deriveVisibleChannelGroups({
  status,
  channels,
  channelRegistry,
  mcpServers,
  mcpRegistry,
  searchQuery,
  t,
}) {
  const builtInChannels = buildBuiltInChannels(status, t).filter((channel) =>
    matchesSearch(searchQuery, [
      t("channels.builtIn"),
      channel.id,
      channel.name,
      channel.description,
      channel.detail,
    ])
  );
  const installedNames = new Set(channels.map((c) => c.name));
  const visibleChannels = channels.filter((channel) =>
    matchesSearch(searchQuery, [
      t("channels.messaging"),
      channel.name,
      channel.display_name,
      channel.description,
      channel.onboarding_state,
    ])
  );
  const availableRegistry = channelRegistry
    .filter((r) => !installedNames.has(r.name))
    .filter((entry) =>
      matchesSearch(searchQuery, [
        t("channels.messaging"),
        entry.name,
        entry.display_name,
        entry.description,
      ])
    );
  const installedMcpNames = new Set(mcpServers.map((m) => m.name));
  const visibleMcpServers = mcpServers.filter((server) =>
    matchesSearch(searchQuery, [
      t("channels.mcpServers"),
      server.name,
      server.display_name,
      server.description,
      server.active ? t("channels.active") : t("channels.inactive"),
    ])
  );
  const availableMcp = mcpRegistry
    .filter((r) => !installedMcpNames.has(r.name))
    .filter((entry) =>
      matchesSearch(searchQuery, [
        t("channels.mcpServers"),
        entry.name,
        entry.display_name,
        entry.description,
      ])
    );

  return {
    builtInChannels,
    visibleChannels,
    availableRegistry,
    visibleMcpServers,
    availableMcp,
  };
}

export function ChannelsTab({ searchQuery = "" }) {
  const t = useT();
  const {
    status,
    channels,
    channelRegistry,
    mcpServers,
    mcpRegistry,
    isLoading,
  } = useChannels();

  if (isLoading) {
    return html`
      <div className="space-y-5">
        <${Card} padding="md">
          <div className="mb-4 h-3 w-28 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
          ${[1, 2, 3].map(
            (i) => html`
              <div
                key=${i}
                className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-4 first:border-0"
              >
                <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
                <div className="h-6 w-16 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
              </div>
            `
          )}
        <//>
      </div>
    `;
  }

  const {
    builtInChannels,
    visibleChannels,
    availableRegistry,
    visibleMcpServers,
    availableMcp,
  } = deriveVisibleChannelGroups({
    status,
    channels,
    channelRegistry,
    mcpServers,
    mcpRegistry,
    searchQuery,
    t,
  });

  if (
    builtInChannels.length === 0 &&
    visibleChannels.length === 0 &&
    availableRegistry.length === 0 &&
    visibleMcpServers.length === 0 &&
    availableMcp.length === 0
  ) {
    return html`<${SettingsSearchEmpty} query=${searchQuery} />`;
  }

  return html`
    <div className="space-y-5">
      ${builtInChannels.length > 0 &&
      html`
      <${Card} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("channels.builtIn")}
        </h3>
        ${builtInChannels.map(
          (channel) => html`
            <${BuiltinChannelCard}
              key=${channel.id}
              name=${channel.name}
              description=${channel.description}
              enabled=${channel.enabled}
              detail=${channel.detail}
            />
          `
        )}
      <//>
      `}

      ${(visibleChannels.length > 0 || availableRegistry.length > 0) &&
      html`
        <${Card} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.messaging")}
          </h3>
          ${visibleChannels.map(
            (ch) => html`
              <${ExtensionChannelCard}
                key=${ch.name}
                channel=${ch}
                registryEntry=${channelRegistry.find((r) => r.name === ch.name)}
              />
            `
          )}
          ${availableRegistry.map(
            (r) => html`
              <${ExtensionChannelCard} key=${r.name} registryEntry=${r} />
            `
          )}
        <//>
      `}
      ${(visibleMcpServers.length > 0 || availableMcp.length > 0) &&
      html`
        <${Card} padding="md">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("channels.mcpServers")}
          </h3>
          ${visibleMcpServers.map(
            (m) =>
              html`
                <div
                  key=${m.name}
                  className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-[var(--v2-text)]"
                        >${m.display_name || m.name}</span
                      >
                      <${Badge}
                        tone=${m.active ? "positive" : "muted"}
                        label=${m.active
                          ? t("channels.active")
                          : t("channels.inactive")}
                        size="sm"
                      />
                    </div>
                    <div className="mt-1 text-xs text-[var(--v2-text-muted)]">
                      ${m.description || ""}
                    </div>
                  </div>
                </div>
              `
          )}
          ${availableMcp.map(
            (r) =>
              html`
                <div
                  key=${r.name}
                  className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-4 first:border-0 first:pt-0"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-[var(--v2-text)]"
                        >${r.display_name || r.name}</span
                      >
                      <${Badge}
                        tone="muted"
                        label=${t("channels.available")}
                        size="sm"
                      />
                    </div>
                    <div className="mt-1 text-xs text-[var(--v2-text-muted)]">
                      ${r.description || ""}
                    </div>
                  </div>
                </div>
              `
          )}
        <//>
      `}
    </div>
  `;
}
