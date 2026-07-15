import React from "react";
import { Button } from "../../../design-system/button";
import { Input } from "../../../design-system/input";
import { Modal, ModalBody, ModalFooter } from "../../../design-system/modal";
import { SelectMenu } from "../../../design-system/select-menu";
import { useT } from "../../../lib/i18n";
import {
  ADAPTER_OPTIONS,
  adapterLabel,
} from "../lib/llm-providers";
import { useProviderDialogForm } from "../hooks/useProviderDialogForm";

export function ProviderDialog({
  provider,
  allProviderIds,
  builtinOverrides,
  open,
  onClose,
  onSave,
  onTest,
  onListModels,
}) {
  const t = useT();
  const formState = useProviderDialogForm({
    provider,
    allProviderIds,
    builtinOverrides,
    open,
    onClose,
    onSave,
    onTest,
    onListModels,
    t,
  });

  const { form, apiKey, models, message, busy, isBuiltin, isEditing } = formState;
  const modelOptions = React.useMemo(() => {
    const typedModel = String(form.model || "").trim();
    const fetchedOptions = models.map((model) => ({ value: model, label: model }));
    if (!typedModel || models.includes(typedModel)) return fetchedOptions;
    return [{ value: typedModel, label: typedModel }, ...fetchedOptions];
  }, [form.model, models]);

  if (!open) return null;

  const title = isBuiltin
    ? t("llm.configureProvider", { name: provider.name || provider.id })
    : isEditing
    ? t("llm.editProvider")
    : t("llm.newProvider");

  return (
    <Modal open={open} onClose={onClose} title={title} size="lg" closeLabel={t("common.close")}>
      <ModalBody className="space-y-4">
        {!isBuiltin &&
        (
          <>
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              {t("llm.providerName")}
              <Input value={form.name} onChange={(e) => formState.update("name", e.currentTarget.value)} />
            </label>
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              {t("llm.providerId")}
              <Input
                value={form.id}
                disabled={isEditing}
                onChange={(e) => {
                  formState.markIdEdited();
                  formState.update("id", e.currentTarget.value);
                }}
              />
            </label>
          </div>
          <div className="space-y-2 text-sm text-[var(--v2-text-strong)]">
            <div>{t("llm.adapter")}</div>
            <SelectMenu
              value={form.adapter}
              options={ADAPTER_OPTIONS}
              onChange={(value) => formState.update("adapter", value)}
              ariaLabel={t("llm.adapter")}
              className="w-full"
              buttonClassName="h-[44px] rounded-[14px] border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3.5 font-sans text-[13px] text-[var(--v2-text-strong)] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm"
            />
          </div>
          </>
        )}

        {isBuiltin &&
        (
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            {adapterLabel(provider.adapter)}
          </div>
        )}

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          {t("llm.baseUrl")}
          <Input value={form.baseUrl} placeholder={provider?.base_url || ""} onChange={(e) => formState.update("baseUrl", e.currentTarget.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          {t("llm.apiKey")}
          <Input type="password" value={apiKey} placeholder={t("llm.apiKeyPlaceholder")} onChange={(e) => formState.setApiKey(e.currentTarget.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          {t("llm.defaultModel")}
          <div className="flex items-stretch gap-2">
            <Input value={form.model} onChange={(e) => formState.update("model", e.currentTarget.value)} />
            <Button type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled={busy !== ""} onClick={formState.fetchModels}>
              {busy === "models" ? t("llm.fetchingModels") : t("llm.fetchModels")}
            </Button>
          </div>
        </label>

        {models.length > 0 &&
        (
          <SelectMenu
            value={form.model}
            options={modelOptions}
            onChange={(value) => formState.update("model", value)}
            ariaLabel={t("llm.defaultModel")}
            className="w-full"
            buttonClassName="h-[44px] rounded-[14px] border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3.5 font-sans text-[13px] text-[var(--v2-text-strong)] md:h-[50px] md:rounded-[16px] md:px-4 md:text-sm"
            menuClassName="!top-auto bottom-[calc(100%+0.35rem)] max-h-64 overflow-y-auto"
          />
        )}

        {message &&
        (
          <div className={message.tone === "error" ? "text-sm text-red-200" : "text-sm text-mint"} role="status">
            {message.text}
          </div>
        )}
      </ModalBody>
      <ModalFooter>
        <Button type="button" variant="secondary" disabled={busy !== ""} onClick={formState.runTest}>
          {busy === "test" ? t("llm.testing") : t("llm.testConnection")}
        </Button>
        <Button type="button" variant="ghost" disabled={busy !== ""} onClick={onClose}>{t("common.cancel")}</Button>
        <Button type="button" disabled={busy !== ""} onClick={formState.submit}>
          {busy === "save" ? t("common.saving") : t("common.save")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}
