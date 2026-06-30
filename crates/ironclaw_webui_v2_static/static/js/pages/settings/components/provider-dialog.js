import { Button } from "../../../design-system/button.js";
import { Input, Select } from "../../../design-system/input.js";
import { Modal, ModalBody, ModalFooter } from "../../../design-system/modal.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import {
  ADAPTER_OPTIONS,
  adapterLabel,
} from "../lib/llm-providers.js";
import { useProviderDialogForm } from "../hooks/useProviderDialogForm.js";

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

  if (!open) return null;

  const { form, apiKey, models, message, busy, isBuiltin, isEditing } = formState;
  const title = isBuiltin
    ? t("llm.configureProvider", { name: provider.name || provider.id })
    : isEditing
    ? t("llm.editProvider")
    : t("llm.newProvider");

  return html`
    <${Modal} open=${open} onClose=${onClose} title=${title} size="lg">
      <${ModalBody} className="space-y-4">
        ${!isBuiltin &&
        html`
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${t("llm.providerName")}
              <${Input} value=${form.name} onChange=${(e) => formState.update("name", e.target.value)} />
            </label>
            <label className="space-y-2 text-sm text-[var(--v2-text-strong)]">
              ${t("llm.providerId")}
              <${Input}
                value=${form.id}
                disabled=${isEditing}
                onChange=${(e) => {
                  formState.markIdEdited();
                  formState.update("id", e.target.value);
                }}
              />
            </label>
          </div>
          <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
            ${t("llm.adapter")}
            <${Select} value=${form.adapter} onChange=${(e) => formState.update("adapter", e.target.value)}>
              ${ADAPTER_OPTIONS.map((item) => html`<option key=${item.value} value=${item.value}>${item.label}</option>`)}
            <//>
          </label>
        `}

        ${isBuiltin &&
        html`
          <div className="rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 text-sm text-[var(--v2-text-muted)]">
            ${adapterLabel(provider.adapter)}
          </div>
        `}

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${t("llm.baseUrl")}
          <${Input} value=${form.baseUrl} placeholder=${provider?.base_url || ""} onChange=${(e) => formState.update("baseUrl", e.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${t("llm.apiKey")}
          <${Input} type="password" value=${apiKey} placeholder=${t("llm.apiKeyPlaceholder")} onChange=${(e) => formState.setApiKey(e.target.value)} />
        </label>

        <label className="block space-y-2 text-sm text-[var(--v2-text-strong)]">
          ${t("llm.defaultModel")}
          <div className="flex items-stretch gap-2">
            <${Input} value=${form.model} onChange=${(e) => formState.update("model", e.target.value)} />
            <${Button} type="button" variant="secondary" className="shrink-0 whitespace-nowrap" disabled=${busy !== ""} onClick=${formState.fetchModels}>
              ${busy === "models" ? t("llm.fetchingModels") : t("llm.fetchModels")}
            <//>
          </div>
        </label>

        ${models.length > 0 &&
        html`
          <${Select} value=${form.model} onChange=${(e) => formState.update("model", e.target.value)}>
            ${models.map((model) => html`<option key=${model} value=${model}>${model}</option>`)}
          <//>
        `}

        ${message &&
        html`
          <div className=${message.tone === "error" ? "text-sm text-red-200" : "text-sm text-mint"} role="status">
            ${message.text}
          </div>
        `}
      <//>
      <${ModalFooter}>
        <${Button} type="button" variant="secondary" disabled=${busy !== ""} onClick=${formState.runTest}>
          ${busy === "test" ? t("llm.testing") : t("llm.testConnection")}
        <//>
        <${Button} type="button" variant="ghost" disabled=${busy !== ""} onClick=${onClose}>${t("common.cancel")}<//>
        <${Button} type="button" disabled=${busy !== ""} onClick=${formState.submit}>
          ${busy === "save" ? t("common.saving") : t("common.save")}
        <//>
      <//>
    <//>
  `;
}
