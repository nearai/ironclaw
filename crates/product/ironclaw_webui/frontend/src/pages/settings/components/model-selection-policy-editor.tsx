// @ts-nocheck
import React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../../design-system/button";
import { Card } from "../../../design-system/card";
import { Input } from "../../../design-system/input";
import { SelectMenu } from "../../../design-system/select-menu";
import { ApiError } from "../../../lib/api";
import { useT } from "../../../lib/i18n";
import { setUserModelPolicy } from "../lib/settings-api";

const MAX_POLICY_MODELS = 128;
const MAX_MODEL_ID_BYTES = 256;

function normalizedModels(models) {
  const seen = new Set();
  const normalized = [];
  for (const value of models || []) {
    const model = String(value || "").trim();
    if (!model || seen.has(model)) continue;
    seen.add(model);
    normalized.push(model);
  }
  return normalized;
}

function validModelId(model) {
  const normalized = String(model || "").trim();
  return (
    normalized.length > 0 &&
    new TextEncoder().encode(normalized).length <= MAX_MODEL_ID_BYTES &&
    !Array.from(normalized).some((character) => /[\u0000-\u001f\u007f]/u.test(character))
  );
}

function modelTestId(model) {
  return String(model).replace(/[^a-zA-Z0-9_-]+/g, "-");
}

function requestErrorMessage(error, fallback) {
  return error instanceof ApiError ? error.message : fallback;
}

export function ModelSelectionPolicyEditor({ providerState }) {
  const t = useT();
  const queryClient = useQueryClient();
  const activeProvider = providerState.providers.find(
    (provider) => provider.id === providerState.activeProviderId
  );
  const activeModel = String(
    providerState.selectedModel || activeProvider?.default_model || ""
  ).trim();
  const policy = providerState.userModelPolicy;
  const policySignature = JSON.stringify([
    providerState.activeProviderId,
    policy?.provider_id,
    policy?.workspace_default,
    policy?.allowed_models || [],
    activeModel,
  ]);
  const [allowedModels, setAllowedModels] = React.useState([]);
  const [workspaceDefault, setWorkspaceDefault] = React.useState("");
  const [discoveredModels, setDiscoveredModels] = React.useState([]);
  const [manualModel, setManualModel] = React.useState("");
  const [status, setStatus] = React.useState(null);

  React.useEffect(() => {
    const policyMatchesActive =
      policy && policy.provider_id === providerState.activeProviderId;
    const initialAllowed = normalizedModels(
      policyMatchesActive ? policy.allowed_models : activeModel ? [activeModel] : []
    );
    setAllowedModels(initialAllowed);
    setWorkspaceDefault(
      policyMatchesActive ? policy.workspace_default : initialAllowed[0] || ""
    );
    setDiscoveredModels(normalizedModels([...initialAllowed, activeModel]));
    setManualModel("");
    setStatus(null);
  }, [policySignature]);

  const saveMutation = useMutation({
    mutationFn: setUserModelPolicy,
    onSuccess: (catalog, request) => {
      queryClient.setQueryData(["user-model-catalog"], catalog);
      queryClient.setQueryData(["llm-providers"], (snapshot) =>
        snapshot
          ? {
              ...snapshot,
              user_model_policy: {
                provider_id: providerState.activeProviderId,
                workspace_default:
                  catalog.workspace_default ?? request.workspace_default,
                allowed_models: catalog.models ?? request.allowed_models,
              },
            }
          : snapshot
      );
      setAllowedModels(catalog.models ?? request.allowed_models);
      setWorkspaceDefault(catalog.workspace_default ?? request.workspace_default);
      setStatus({ tone: "success", text: t("llm.policyEnabled") });
    },
    onError: (error) => {
      setStatus({
        tone: "error",
        text: requestErrorMessage(error, t("llm.policySaveFailed")),
      });
    },
  });

  const candidates = normalizedModels([
    ...discoveredModels,
    ...allowedModels,
    activeModel,
  ]);
  const defaultOptions = allowedModels.map((model) => ({ value: model, label: model }));

  const addManualModel = () => {
    const model = manualModel.trim();
    if (!validModelId(model)) {
      setStatus({ tone: "error", text: t("llm.policyInvalidModel") });
      return;
    }
    if (!allowedModels.includes(model) && allowedModels.length >= MAX_POLICY_MODELS) {
      setStatus({ tone: "error", text: t("llm.policyTooManyModels") });
      return;
    }
    setDiscoveredModels((current) => normalizedModels([...current, model]));
    setAllowedModels((current) => normalizedModels([...current, model]));
    setWorkspaceDefault((current) => current || model);
    setManualModel("");
    setStatus(null);
  };

  const toggleModel = (model, checked) => {
    setAllowedModels((current) => {
      if (checked && !current.includes(model) && current.length >= MAX_POLICY_MODELS) {
        setStatus({ tone: "error", text: t("llm.policyTooManyModels") });
        return current;
      }
      const next = checked
        ? normalizedModels([...current, model])
        : current.filter((candidate) => candidate !== model);
      setWorkspaceDefault((currentDefault) =>
        next.includes(currentDefault) ? currentDefault : next[0] || ""
      );
      return next;
    });
    setStatus(null);
  };

  const fetchModels = async () => {
    if (!activeProvider) return;
    setStatus(null);
    try {
      const result = await providerState.listModels({
        provider_id: activeProvider.id,
        adapter: activeProvider.adapter,
        base_url: activeProvider.base_url || undefined,
        model: activeModel || undefined,
      });
      if (!result.ok || !Array.isArray(result.models) || result.models.length === 0) {
        setStatus({
          tone: "error",
          text: result.message || t("llm.modelsFetchFailed"),
        });
        return;
      }
      setDiscoveredModels((current) => normalizedModels([...current, ...result.models]));
      setStatus({
        tone: "success",
        text: t("llm.modelsFetched", { count: result.models.length }),
      });
    } catch (error) {
      setStatus({
        tone: "error",
        text: requestErrorMessage(error, t("llm.modelsLoadFailed")),
      });
    }
  };

  const savePolicy = () => {
    if (!providerState.activeProviderId) {
      setStatus({ tone: "error", text: t("llm.policyNoActiveProvider") });
      return;
    }
    if (allowedModels.length === 0 || !allowedModels.includes(workspaceDefault)) {
      setStatus({ tone: "error", text: t("llm.policySelectModels") });
      return;
    }
    saveMutation.mutate({
      workspace_default: workspaceDefault,
      allowed_models: allowedModels,
    });
  };

  const noActiveProvider = !providerState.activeProviderId || !activeProvider;

  return (
    <Card
      padding="none"
      className="p-4 sm:p-5"
      data-testid="settings-model-policy-editor"
    >
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            {t("llm.policyTitle")}
          </h3>
          <p className="mt-2 max-w-2xl text-sm text-[var(--v2-text-muted)]">
            {t("llm.policyDesc")}
          </p>
        </div>
        {activeProvider && (
          <span className="font-mono text-xs text-[var(--v2-text-muted)]">
            {activeProvider.id}
          </span>
        )}
      </div>

      {noActiveProvider ? (
        <p className="text-sm text-[var(--v2-warning-text)]">
          {t("llm.policyNoActiveProvider")}
        </p>
      ) : (
        <div className="space-y-4">
          <div className="flex flex-col gap-2 sm:flex-row">
            <Input
              data-testid="settings-model-policy-model-input"
              size="sm"
              value={manualModel}
              maxLength={MAX_MODEL_ID_BYTES}
              placeholder={t("llm.policyModelPlaceholder")}
              onChange={(event) => setManualModel(event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key !== "Enter") return;
                event.preventDefault();
                addManualModel();
              }}
            />
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="shrink-0 whitespace-nowrap"
              data-testid="settings-model-policy-add-model"
              disabled={!manualModel.trim() || saveMutation.isPending}
              onClick={addManualModel}
            >
              {t("llm.policyAddModel")}
            </Button>
            {activeProvider.can_list_models && (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                className="shrink-0 whitespace-nowrap"
                disabled={saveMutation.isPending}
                onClick={fetchModels}
              >
                {t("llm.fetchModels")}
              </Button>
            )}
          </div>

          <div>
            <div className="mb-2 text-xs font-medium text-[var(--v2-text-muted)]">
              {t("llm.policyAllowedModels")}
            </div>
            <div className="max-h-52 space-y-1 overflow-y-auto rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-2">
              {candidates.map((model) => (
                <label
                  key={model}
                  className="flex cursor-pointer items-center gap-3 rounded px-2 py-2 text-sm text-[var(--v2-text-strong)] hover:bg-white/[0.04]"
                >
                  <input
                    type="checkbox"
                    data-testid={`settings-model-policy-model-${modelTestId(model)}`}
                    checked={allowedModels.includes(model)}
                    disabled={saveMutation.isPending}
                    onChange={(event) => toggleModel(model, event.currentTarget.checked)}
                  />
                  <span className="min-w-0 break-all font-mono text-xs">{model}</span>
                </label>
              ))}
            </div>
          </div>

          <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end">
            <div>
              <div className="mb-2 text-xs font-medium text-[var(--v2-text-muted)]">
                {t("llm.policyWorkspaceDefault")}
              </div>
              <SelectMenu
                value={workspaceDefault}
                options={defaultOptions}
                onChange={setWorkspaceDefault}
                disabled={allowedModels.length === 0 || saveMutation.isPending}
                ariaLabel={t("llm.policyWorkspaceDefault")}
                className="w-full"
                buttonClassName="w-full"
                align="left"
              />
            </div>
            <Button
              type="button"
              data-testid="settings-model-policy-save"
              disabled={
                noActiveProvider ||
                allowedModels.length === 0 ||
                !workspaceDefault ||
                saveMutation.isPending
              }
              onClick={savePolicy}
            >
              {saveMutation.isPending ? t("common.saving") : t("common.save")}
            </Button>
          </div>
        </div>
      )}

      <div
        data-testid="settings-model-policy-status"
        className={[
          "mt-3 min-h-5 text-xs",
          status?.tone === "error"
            ? "text-[var(--v2-danger-text)]"
            : status?.tone === "success"
              ? "text-[var(--v2-positive-text)]"
              : "text-[var(--v2-text-muted)]",
        ].join(" ")}
        role="status"
      >
        {status?.text ||
          (policy?.provider_id === providerState.activeProviderId
            ? t("llm.policyEnabled")
            : t("llm.policyDisabled"))}
      </div>
    </Card>
  );
}
