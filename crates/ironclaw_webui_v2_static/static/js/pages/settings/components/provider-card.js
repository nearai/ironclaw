import { Button } from "../../../design-system/button.js";
import { Badge } from "../../../design-system/badge.js";
import { Card } from "../../../design-system/card.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import {
  adapterLabel,
  isProviderConfigured,
  providerDisplayModel,
  providerEffectiveBaseUrl,
} from "../lib/llm-providers.js";

export function ProviderCard({
  provider,
  activeProviderId,
  selectedModel,
  builtinOverrides,
  isBusy,
  onUse,
  onConfigure,
  onDelete,
}) {
  const t = useT();
  const isActive = provider.id === activeProviderId;
  const configured = isProviderConfigured(provider, builtinOverrides);
  const baseUrl = providerEffectiveBaseUrl(provider, builtinOverrides);
  const model = providerDisplayModel(provider, builtinOverrides, activeProviderId, selectedModel);

  return html`
    <${Card}
      className=${[
        "p-4 sm:p-5",
        isActive ? "border-[color-mix(in_srgb,var(--v2-accent)_55%,var(--v2-panel-border))]" : "",
      ].join(" ")}
    >
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h4 className="min-w-0 truncate text-base font-semibold text-[var(--v2-text-strong)]">
              ${provider.name || provider.id}
            </h4>
            <span className="font-mono text-xs text-[var(--v2-text-faint)]">${provider.id}</span>
            ${isActive && html`<${Badge} tone="positive" label=${t("llm.active")} size="sm" />`}
            ${provider.builtin && html`<${Badge} tone="muted" label=${t("llm.builtin")} size="sm" />`}
            ${!isActive && !configured &&
            html`<${Badge} tone="warning" label=${t("llm.notConfigured")} size="sm" />`}
          </div>

          <div className="mt-3 grid gap-2 text-xs text-[var(--v2-text-muted)] sm:grid-cols-3">
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${t("llm.adapter")}</div>
              <div className="mt-1 truncate">${adapterLabel(provider.adapter)}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${t("llm.baseUrl")}</div>
              <div className="mt-1 truncate font-mono">${baseUrl || t("llm.none")}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${t("llm.model")}</div>
              <div className="mt-1 truncate font-mono">${model || t("llm.none")}</div>
            </div>
          </div>
        </div>

        <div className="flex shrink-0 flex-wrap gap-2">
          ${!isActive &&
          html`
            <${Button}
              type="button"
              variant=${configured ? "primary" : "secondary"}
              size="sm"
              disabled=${isBusy}
              onClick=${() => (configured ? onUse(provider) : onConfigure(provider))}
            >
              ${configured ? t("llm.use") : t("llm.configure")}
            <//>
          `}
          ${(provider.builtin && provider.id !== "bedrock") || !provider.builtin
            ? html`
                <${Button}
                  type="button"
                  variant="secondary"
                  size="sm"
                  disabled=${isBusy}
                  onClick=${() => onConfigure(provider)}
                >
                  ${provider.builtin ? t("llm.configure") : t("common.edit")}
                <//>
              `
            : null}
          ${!provider.builtin &&
          !isActive &&
          html`
            <${Button}
              type="button"
              variant="danger"
              size="sm"
              disabled=${isBusy}
              onClick=${() => onDelete(provider)}
            >
              ${t("common.delete")}
            <//>
          `}
        </div>
      </div>
    <//>
  `;
}
