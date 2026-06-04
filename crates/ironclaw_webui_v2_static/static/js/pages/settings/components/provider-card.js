import { Button } from "../../../design-system/button.js";
import { Badge } from "../../../design-system/badge.js";
import { Card } from "../../../design-system/card.js";
import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import {
  adapterLabel,
  isProviderConfigured,
  providerDisplayModel,
  providerEffectiveBaseUrl,
  providerMissingReason,
} from "../lib/llm-providers.js";

/**
 * One row in the grouped LLM-providers list.
 *
 * Visual: compact name + status pill + primary action + chevron, with the
 * adapter / base-URL / model grid hidden behind an inline expand toggle.
 * The active provider expands by default so its config is visible without
 * an extra click.
 *
 * Markup contract:
 *   - The row is a `<div role="button">` not a `<button>` because it
 *     embeds other interactive elements (primary action button, chevron
 *     toggle). Nested `<button>`s are invalid HTML and break clicks in
 *     some browsers; the `role` + `tabIndex` + keydown handler give us
 *     equivalent keyboard semantics without the parser hazard.
 *   - Action buttons stop propagation so clicking "Use" / "Configure"
 *     does not also toggle the disclosure.
 *
 * Behavioral parity with the old card: `onUse`, `onConfigure`, `onDelete`
 * are unchanged so all upstream wiring (provider dialog, activation flow,
 * delete confirmation) keeps working.
 */
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
  const missing = providerMissingReason(provider, builtinOverrides);

  // Active row defaults to expanded — user almost always wants to see the
  // running model. Other rows start collapsed and reveal on demand.
  const [expanded, setExpanded] = React.useState(isActive);
  const toggle = React.useCallback(() => setExpanded((v) => !v), []);

  // When a different row gets activated via the Use flow we want it to
  // auto-expand even if the user had it collapsed before. Without this
  // effect a row that was rendered collapsed before becoming active would
  // stay collapsed despite the "active row expands by default" rule.
  React.useEffect(() => {
    if (isActive) setExpanded(true);
  }, [isActive]);

  const onKeyDown = React.useCallback(
    (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        toggle();
      }
    },
    [toggle]
  );

  // Inline meta line shown on the collapsed row: adapter · model when
  // configured, "Missing X" when something blocks activation. Keeps the row
  // scannable without forcing the user to expand to see "why can't I use this".
  const inlineMeta = !configured
    ? html`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${missing === "api_key" ? t("llm.missingApiKey") : t("llm.missingBaseUrl")}
      </span>`
    : html`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${adapterLabel(provider.adapter)} · ${model || provider.default_model || t("llm.none")}
      </span>`;

  // Primary action: "Use" for ready providers, "Configure" / "Add API key" for
  // not-yet-configured ones, nothing for the already-active row.
  const primaryAction = isActive
    ? null
    : configured
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
    : html`
        <${Button}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${isBusy}
          onClick=${() => onConfigure(provider)}
        >
          ${missing === "api_key" ? t("llm.addApiKey") : t("llm.configure")}
        <//>
      `;

  const stop = React.useCallback((e) => e.stopPropagation(), []);

  return html`
    <${Card}
      padding="none"
      className=${[
        "transition-colors",
        isActive
          ? "border-[color-mix(in_srgb,var(--v2-positive-text)_36%,var(--v2-panel-border))]"
          : expanded
          ? "border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]"
          : "",
      ].join(" ")}
    >
      <div
        role="button"
        tabIndex=${0}
        aria-expanded=${expanded}
        aria-label=${expanded ? t("llm.collapseDetails") : t("llm.expandDetails")}
        onClick=${toggle}
        onKeyDown=${onKeyDown}
        className="flex w-full cursor-pointer items-center gap-3 px-4 py-3 hover:bg-[var(--v2-surface-soft)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)] sm:px-5"
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
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          <span className="min-w-0 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
            ${provider.name || provider.id}
          </span>
          <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${provider.id}</span>
          ${isActive && html`<${Badge} tone="positive" label=${t("llm.active")} size="sm" />`}
          ${provider.builtin && !isActive &&
          html`<${Badge} tone="muted" label=${t("llm.builtin")} size="sm" />`}
        </div>
        <div className="hidden min-w-0 max-w-[280px] truncate sm:block">${inlineMeta}</div>
        <div
          className="flex shrink-0 items-center gap-2"
          onClick=${stop}
          onKeyDown=${stop}
        >
          ${primaryAction}
          <button
            type="button"
            onClick=${toggle}
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
        <div className="border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-4 sm:px-5">
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
            ${/* Edit/Configure button — built-ins always show Configure,
                custom providers show Edit. Bedrock has no configurable
                surface beyond credentials handled elsewhere, so we omit it
                like the previous design did. */
            ((provider.builtin && provider.id !== "bedrock") || !provider.builtin) &&
            html`
              <${Button}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${isBusy}
                onClick=${() => onConfigure(provider)}
              >
                ${provider.builtin ? t("llm.configure") : t("common.edit")}
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
