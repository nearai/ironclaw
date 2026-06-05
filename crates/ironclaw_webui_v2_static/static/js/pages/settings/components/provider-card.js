import { Button } from "../../../design-system/button.js";
import { Badge } from "../../../design-system/badge.js";
import { Card } from "../../../design-system/card.js";
import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import {
  adapterLabel,
  isProviderConfigured,
  providerAcceptsApiKey,
  providerDisplayModel,
  providerEffectiveBaseUrl,
  providerMissingReason,
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
  onNearaiLogin,
  onNearaiWallet,
  onCodexLogin,
  loginBusy,
}) {
  const t = useT();
  const isActive = provider.id === activeProviderId;
  const configured = isProviderConfigured(provider, builtinOverrides);
  const baseUrl = providerEffectiveBaseUrl(provider, builtinOverrides);
  const model = providerDisplayModel(provider, builtinOverrides, activeProviderId, selectedModel);
  const missing = providerMissingReason(provider, builtinOverrides);
  const acceptsApiKey = providerAcceptsApiKey(provider);
  const missingLabel =
    missing === "api_key"
      ? t("llm.missingApiKey")
      : missing === "base_url"
      ? t("llm.missingBaseUrl")
      : t("llm.notConfigured");

  const [expanded, setExpanded] = React.useState(isActive);
  const toggle = React.useCallback(() => setExpanded((v) => !v), []);

  React.useEffect(() => {
    setExpanded(isActive);
  }, [isActive]);

  const inlineMeta = !configured
    ? html`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${missingLabel}
      </span>`
    : html`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${adapterLabel(provider.adapter)} · ${model || provider.default_model || t("llm.none")}
      </span>`;

  const isLoginProvider = provider.id === "nearai" || provider.id === "openai_codex";
  const hasApiKey = provider.api_key_set === true || provider.has_api_key === true;
  const configureLabel = provider.builtin
    ? provider.id === "nearai" && acceptsApiKey && !hasApiKey
      ? t("llm.addApiKey")
      : t("llm.configure")
    : t("common.edit");
  const apiKeyAction =
    acceptsApiKey && provider.builtin
      ? html`
          <${Button}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${isBusy}
            onClick=${() => onConfigure(provider)}
          >
            ${configureLabel}
          <//>
        `
      : null;
  const loginActions =
    !isActive && provider.id === "nearai"
      ? html`
          ${apiKeyAction}
          <${Button} type="button" variant="secondary" size="sm" disabled=${loginBusy} onClick=${onNearaiWallet}>
            ${t("onboarding.nearWallet")}
          <//>
          <${Button} type="button" variant="secondary" size="sm" disabled=${loginBusy} onClick=${() => onNearaiLogin("github")}>
            GitHub
          <//>
          <${Button} type="button" variant="secondary" size="sm" disabled=${loginBusy} onClick=${() => onNearaiLogin("google")}>
            Google
          <//>
        `
      : !isActive && provider.id === "openai_codex"
      ? html`
          <${Button} type="button" variant="secondary" size="sm" disabled=${loginBusy} onClick=${onCodexLogin}>
            ${t("onboarding.codexSignIn")}
          <//>
        `
      : null;
  const canUseProvider =
    !isActive &&
    configured &&
    (!isLoginProvider || (provider.id === "nearai" && provider.has_api_key === true));
  const useAction = canUseProvider
    ? html`
        <${Button}
          type="button"
          variant="primary"
          size="sm"
          disabled=${isBusy}
          onClick=${() => onUse(provider)}
        >
          ${t("llm.use")}
        <//>
      `
    : null;
  const setupAction = !configured
    ? html`
        <${Button}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${isBusy}
          onClick=${() => onConfigure(provider)}
        >
          ${missing === "api_key" ? t("llm.addApiKey") : t("llm.configure")}
        <//>
      `
    : null;
  const primaryAction = isActive
    ? null
    : useAction || (isLoginProvider ? loginActions : setupAction);
  const showConfigureAction =
    (!isLoginProvider && ((provider.builtin && provider.id !== "bedrock") || !provider.builtin)) ||
    (provider.id === "nearai" && acceptsApiKey);

  return html`
    <${Card}
      padding="none"
      data-testid="llm-provider-card"
      data-provider-id=${provider.id}
      className=${[
        "transition-colors",
        isActive
          ? "border-[color-mix(in_srgb,var(--v2-positive-text)_36%,var(--v2-panel-border))]"
          : expanded
          ? "border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]"
          : "",
      ].join(" ")}
    >
      <div className="flex w-full items-stretch hover:bg-[var(--v2-surface-soft)]">
        <button
          type="button"
          aria-expanded=${expanded ? "true" : "false"}
          aria-label=${expanded ? t("llm.collapseDetails") : t("llm.expandDetails")}
          data-testid="llm-provider-disclosure"
          onClick=${toggle}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 px-4 py-3 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)] sm:pl-5 sm:pr-3"
        >
          <span
            className=${[
              "h-2 w-2 shrink-0 rounded-full",
              isActive
                ? "bg-[var(--v2-positive-text)]"
                : configured
                ? "bg-[var(--v2-accent)]"
                : "bg-[var(--v2-warning-text)]",
            ].join(" ")}
          />
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
            <span className="min-w-0 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
              ${provider.name || provider.id}
            </span>
            <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${provider.id}</span>
            ${isActive && html`<${Badge} tone="positive" label=${t("llm.active")} size="sm" />`}
            ${provider.builtin && !isActive &&
            html`<${Badge} tone="muted" label=${t("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${inlineMeta}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 py-3 pr-4 sm:pr-5">
          ${primaryAction}
          <button
            type="button"
            onClick=${toggle}
            data-testid="llm-provider-chevron"
            aria-label=${expanded ? t("llm.collapseDetails") : t("llm.expandDetails")}
            className=${[
              "grid h-7 w-7 place-items-center rounded-md text-[var(--v2-text-faint)] transition-transform hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]",
              expanded ? "rotate-180" : "",
            ].join(" ")}
          >
            <${Icon} name="chevron" className="h-4 w-4" />
          </button>
        </div>
      </div>

      ${expanded &&
      html`
        <div data-testid="llm-provider-details" className="border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-4 sm:px-5">
          <div className="grid gap-3 text-xs text-[var(--v2-text-muted)] sm:grid-cols-3">
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

          <div className="mt-4 flex flex-wrap justify-end gap-2 border-t border-[var(--v2-panel-border)] pt-3">
            ${showConfigureAction &&
            html`
              <${Button}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${isBusy}
                onClick=${() => onConfigure(provider)}
              >
                ${configureLabel}
              <//>
            `}
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
      `}
    <//>
  `;
}
