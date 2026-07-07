import { Button } from "../../../design-system/button.js";
import { Card } from "../../../design-system/card.js";
import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";
import { ProviderCard } from "./provider-card.js";
import { ProviderDialog } from "./provider-dialog.js";
import { ProviderLoginStatus } from "./provider-login-status.js";
import { useProviderManagementActions } from "../hooks/useProviderManagementActions.js";
import { useProviderLogin } from "../hooks/useProviderLogin.js";
import {
  groupProvidersByStatus,
} from "../lib/llm-providers.js";

const GROUP_ORDER = [
  { key: "active", labelKey: "llm.groupActive", dotClass: "bg-[var(--v2-positive-text)]" },
  { key: "ready", labelKey: "llm.groupReady", dotClass: "bg-[var(--v2-accent)]" },
  { key: "setup", labelKey: "llm.groupSetup", dotClass: "bg-[var(--v2-warning-text)]" },
];

const STATUS_DOT = Object.fromEntries(
  GROUP_ORDER.map(({ key, dotClass }) => [key, dotClass])
);

// Single source of truth for flattening grouped providers into a tagged list
// ordered active → ready → setup. Used by both the parent lookup and the
// mobile dropdown to avoid the two derivations drifting apart.
function flattenProvidersByGroup(groups) {
  return GROUP_ORDER.flatMap(({ key }) =>
    groups[key].map((provider) => ({ provider, status: key }))
  );
}

function MobileProviderSelect({
  items,
  selectedId,
  onSelect,
}) {
  const t = useT();
  const detailsRef = React.useRef(null);
  const selected = selectedId
    ? items.find((item) => item.provider.id === selectedId)
    : null;

  // Close the native <details> dropdown once a provider is picked so it
  // does not cover the card it just revealed. Restore focus to the summary
  // trigger so keyboard/screen-reader users keep their place.
  const handleSelect = (id) => {
    onSelect(id);
    if (detailsRef.current) {
      detailsRef.current.removeAttribute("open");
      const summaryEl = detailsRef.current.querySelector("summary");
      if (summaryEl) summaryEl.focus();
    }
  };

  return html`
    <details ref=${detailsRef} className="group mb-3 w-full overflow-hidden sm:hidden">
      <summary
        className="flex w-full min-w-0 cursor-pointer list-none items-center justify-between gap-3 overflow-hidden rounded-[14px] border border-white/10 bg-white/[0.03] px-4 py-3 text-sm text-white [&::-webkit-details-marker]:hidden"
      >
        <span className="flex min-w-0 flex-1 items-start gap-2.5">
          ${selected
            ? html`<span className=${"mt-1.5 h-2 w-2 shrink-0 rounded-full " + STATUS_DOT[selected.status]} />`
            : html`<span className="mt-1.5 h-2 w-2 shrink-0 rounded-full bg-[var(--v2-text-faint)]" />`
          }
          <span className="min-w-0 shrink break-words line-clamp-2 font-semibold leading-snug">
            ${selected
              ? selected.provider.name || selected.provider.id
              : t("llm.selectProvider")
            }
          </span>
          ${selected && selected.status === "active" && html`<span className="mt-0.5 shrink-0 font-mono text-[10px] uppercase tracking-wider text-[var(--v2-positive-text)]">${t("llm.active")}</span>`}
        </span>
        <span
          aria-hidden="true"
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-white/10 bg-white/[0.06] text-[var(--v2-text-muted)] transition-transform duration-150 group-open:rotate-180"
        >
          <${Icon} name="chevron" className="h-4 w-4" />
        </span>
      </summary>
      <div className="mt-2 grid gap-1 overflow-hidden rounded-[14px] border border-white/10 bg-white/[0.03] p-1">
        ${items.map(
          (item) => html`
            <button
              key=${item.provider.id}
              onClick=${() => handleSelect(item.provider.id)}
              className=${[
                "flex w-full min-w-0 items-start gap-3 rounded-[12px] px-3 py-2 text-left text-sm",
                selectedId === item.provider.id
                  ? "bg-signal/10 text-white"
                  : "text-iron-300 hover:bg-white/[0.045] hover:text-white",
              ].join(" ")}
            >
              <span className=${"mt-1.5 h-2 w-2 shrink-0 rounded-full " + STATUS_DOT[item.status]} />
              <span className="min-w-0 shrink break-words line-clamp-2 leading-snug">${item.provider.name || item.provider.id}</span>
              ${item.provider.id === selectedId && html`<${Icon} name="check" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-[var(--v2-accent-text)]" />`}
            </button>
          `
        )}
      </div>
    </details>
  `;
}

function GroupHeader({ label, count, dotClass }) {
  return html`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full " + dotClass} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${label}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${count}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `;
}

export function ProviderManagement({ settings, gatewayStatus, searchQuery = "" }) {
  const t = useT();
  const actions = useProviderManagementActions({ settings, gatewayStatus, searchQuery, t });
  const state = actions.providerState;
  // NEAR AI / Codex authenticate via login flows; on success the snapshot
  // refresh re-renders the now-active card in place (no navigation here).
  const login = useProviderLogin();
  const loginBusy = login.nearaiBusy || login.codexBusy;

  // Mobile: collapse the full provider list into a dropdown. The user picks
  // one provider and sees only that card, instead of scrolling through all
  // of them. Desktop keeps the full grouped list.
  const [mobileSelectedId, setMobileSelectedId] = React.useState(null);

  if (searchQuery && actions.filteredProviders.length === 0) {
    return html`<${SettingsSearchEmpty} query=${searchQuery} />`;
  }

  const groups = groupProvidersByStatus(
    actions.filteredProviders,
    state.builtinOverrides,
    state.activeProviderId
  );

  // Build the tagged mobile list once (single source of truth) — used both
  // for the dropdown rendering and the parent's provider lookup.
  const mobileItems = flattenProvidersByGroup(groups);
  // Default the mobile selection to the active provider so the user sees
  // their configured card immediately, without having to open the dropdown.
  const selectedId = mobileSelectedId || state.activeProviderId;
  const mobileProvider = selectedId
    ? (mobileItems.find((item) => item.provider.id === selectedId)?.provider ?? null)
    : null;

  const renderProviderCard = (provider) => html`
    <${ProviderCard}
      key=${provider.id}
      provider=${provider}
      activeProviderId=${state.activeProviderId}
      selectedModel=${state.selectedModel}
      builtinOverrides=${state.builtinOverrides}
      isBusy=${state.isBusy}
      onUse=${actions.handleUse}
      onConfigure=${actions.openDialog}
      onDelete=${actions.handleDelete}
      onNearaiLogin=${login.startNearai}
      onNearaiWallet=${login.startNearaiWallet}
      onCodexLogin=${login.startCodex}
      loginBusy=${loginBusy}
    />
  `;

  return html`
    <${Card} className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${t("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${t("llm.providersDesc")}</p>
        </div>
        <${Button} type="button" variant="secondary" size="sm" className="gap-2" onClick=${() => actions.openDialog(null)}>
          <${Icon} name="plus" className="h-3.5 w-3.5" />
          ${t("llm.addProvider")}
        <//>
      </div>

      ${actions.message &&
      html`
        <div
          className=${[
            "mb-4 rounded-md border px-3 py-2 text-sm",
            actions.message.tone === "error"
              ? "border-red-400/30 bg-red-500/10 text-red-200"
              : "border-mint/30 bg-mint/10 text-mint",
          ].join(" ")}
          role="status"
        >
          ${actions.message.text}
        </div>
      `}

      <${ProviderLoginStatus} login=${login} />

      ${state.isLoading
        ? html`<div className="text-sm text-[var(--v2-text-muted)]">${t("common.loading")}</div>`
        : state.error
        ? html`<div className="text-sm text-red-200">${t("error.loadFailed", { what: t("llm.providers"), message: state.error.message })}</div>`
        : html`
            <${MobileProviderSelect}
              items=${mobileItems}
              selectedId=${selectedId}
              onSelect=${setMobileSelectedId}
            />
            ${mobileProvider && html`
              <div className="sm:hidden">
                ${renderProviderCard(mobileProvider)}
              </div>
            `}
            <div className="hidden space-y-1 sm:block">
              ${GROUP_ORDER.flatMap((group) => {
                const items = groups[group.key];
                if (!items.length) return [];
                return [
                  html`
                    <section
                      key=${group.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${group.key}
                      className="mb-3"
                    >
                      <${GroupHeader}
                        label=${t(group.labelKey)}
                        count=${items.length}
                        dotClass=${group.dotClass}
                      />
                      <div className="space-y-2">
                      ${items.map(renderProviderCard)}
                      </div>
                    </section>
                  `,
                ];
              })}
            </div>
          `}

      <${ProviderDialog}
        open=${actions.isDialogOpen}
        provider=${actions.dialogProvider}
        allProviderIds=${actions.allProviderIds}
        builtinOverrides=${state.builtinOverrides}
        onClose=${actions.closeDialog}
        onSave=${actions.handleSave}
        onTest=${state.testConnection}
        onListModels=${state.listModels}
      />
    <//>
  `;
}
