import { Button } from "../../../design-system/button";
import { Card } from "../../../design-system/card";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { SettingsSearchEmpty } from "./settings-search-empty";
import { ProviderCard } from "./provider-card";
import { ProviderDialog } from "./provider-dialog";
import { ProviderLoginStatus } from "./provider-login-status";
import { useProviderManagementActions } from "../hooks/useProviderManagementActions";
import { useProviderLogin } from "../hooks/useProviderLogin";
import { groupProvidersByStatus } from "../lib/llm-providers";

const GROUP_ORDER = [
  { key: "active", labelKey: "llm.groupActive", dotClass: "bg-[var(--v2-positive-text)]" },
  { key: "ready", labelKey: "llm.groupReady", dotClass: "bg-[var(--v2-accent)]" },
  { key: "setup", labelKey: "llm.groupSetup", dotClass: "bg-[var(--v2-warning-text)]" },
];

function GroupHeader({ label, count, dotClass }) {
  return (
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className={"h-1.5 w-1.5 rounded-full " + dotClass} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        {label}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">{count}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  );
}

export function ProviderManagement({ settings, gatewayStatus, searchQuery = "" }) {
  const t = useT();
  const actions = useProviderManagementActions({ settings, gatewayStatus, searchQuery, t });
  const state = actions.providerState;
  // NEAR AI / Codex authenticate via login flows; on success the snapshot
  // refresh re-renders the now-active card in place (no navigation here).
  const login = useProviderLogin();
  const loginBusy = login.nearaiBusy || login.codexBusy;

  if (searchQuery && actions.filteredProviders.length === 0) {
    return (<SettingsSearchEmpty query={searchQuery} />);
  }

  const groups = groupProvidersByStatus(
    actions.filteredProviders,
    state.builtinOverrides,
    state.activeProviderId
  );

  return (
    <Card className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            {t("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">{t("llm.providersDesc")}</p>
        </div>
        <Button type="button" variant="secondary" size="sm" className="gap-2" onClick={() => actions.openDialog(null)}>
          <Icon name="plus" className="h-3.5 w-3.5" />
          {t("llm.addProvider")}
        </Button>
      </div>

      {actions.message &&
      (
        <div
          className={[
            "mb-4 rounded-md border px-3 py-2 text-sm",
            actions.message.tone === "error"
              ? "border-red-400/30 bg-red-500/10 text-red-200"
              : "border-mint/30 bg-mint/10 text-mint",
          ].join(" ")}
          role="status"
        >
          {actions.message.text}
        </div>
      )}

      <ProviderLoginStatus login={login} />

      {state.isLoading
        ? (<div className="text-sm text-[var(--v2-text-muted)]">{t("common.loading")}</div>)
        : state.error
        ? (<div className="text-sm text-red-200">{t("error.loadFailed", { what: t("llm.providers"), message: state.error.message })}</div>)
        : (
            <div className="space-y-1">
              {GROUP_ORDER.flatMap((group) => {
                const items = groups[group.key];
                if (!items.length) return [];
                return [
                  (
                    <section
                      key={group.key}
                      data-testid="llm-provider-group"
                      data-provider-status={group.key}
                      className="mb-3"
                    >
                      <GroupHeader
                        label={t(group.labelKey)}
                        count={items.length}
                        dotClass={group.dotClass}
                      />
                      <div className="space-y-2">
                      {items.map(
                        (provider) => (
                          <ProviderCard
                            key={provider.id}
                            provider={provider}
                            activeProviderId={state.activeProviderId}
                            selectedModel={state.selectedModel}
                            builtinOverrides={state.builtinOverrides}
                            isBusy={state.isBusy}
                            onUse={actions.handleUse}
                            onConfigure={actions.openDialog}
                            onDelete={actions.handleDelete}
                            onNearaiLogin={login.startNearai}
                            onNearaiWallet={login.startNearaiWallet}
                            onCodexLogin={login.startCodex}
                            loginBusy={loginBusy}
                          />
                        )
                      )}
                      </div>
                    </section>
                  ),
                ];
              })}
            </div>
          )}

      <ProviderDialog
        open={actions.isDialogOpen}
        provider={actions.dialogProvider}
        allProviderIds={actions.allProviderIds}
        builtinOverrides={state.builtinOverrides}
        onClose={actions.closeDialog}
        onSave={actions.handleSave}
        onTest={state.testConnection}
        onListModels={state.listModels}
      />
    </Card>
  );
}
