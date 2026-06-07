import { React } from "../../../lib/html.js";
import {
  isValidProviderId,
  providerEffectiveBaseUrl,
  providerIdFromName,
  providerDefaultModel,
  providerPayload,
} from "../lib/llm-providers.js";

function initialForm(provider, overrides) {
  return {
    name: provider?.name || "",
    id: provider?.id || "",
    adapter: provider?.adapter || "open_ai_completions",
    baseUrl: provider ? providerEffectiveBaseUrl(provider, overrides) : "",
    model: provider ? providerDefaultModel(provider, overrides) : "",
  };
}

export function useProviderDialogForm({
  provider,
  allProviderIds,
  builtinOverrides,
  open,
  onClose,
  onSave,
  onTest,
  onListModels,
  t,
}) {
  const [form, setForm] = React.useState(() => initialForm(provider, builtinOverrides));
  const [apiKey, setApiKey] = React.useState("");
  const [models, setModels] = React.useState([]);
  const [message, setMessage] = React.useState(null);
  const [busy, setBusy] = React.useState("");
  const idEditedRef = React.useRef(Boolean(provider));

  React.useEffect(() => {
    if (!open) return;
    setForm(initialForm(provider, builtinOverrides));
    setApiKey("");
    setModels([]);
    setMessage(null);
    setBusy("");
    idEditedRef.current = Boolean(provider);
  }, [open, provider, builtinOverrides]);

  const isBuiltin = provider?.builtin === true;
  const isEditing = provider && !provider.builtin;

  const update = React.useCallback((key, value) => {
    setForm((prev) => {
      const next = { ...prev, [key]: value };
      if (key === "name" && !idEditedRef.current) next.id = providerIdFromName(value);
      return next;
    });
  }, []);

  const validate = React.useCallback(() => {
    if (!isBuiltin && (!form.name.trim() || !form.id.trim())) return t("llm.fieldsRequired");
    if (!isBuiltin && !isValidProviderId(form.id.trim())) return t("llm.invalidId");
    if (!isEditing && !isBuiltin && allProviderIds.includes(form.id.trim())) {
      return t("llm.idTaken", { id: form.id.trim() });
    }
    return "";
  }, [allProviderIds, form.id, form.name, isBuiltin, isEditing, t]);

  const submit = React.useCallback(async () => {
    const error = validate();
    if (error) {
      setMessage({ tone: "error", text: error });
      return;
    }
    setBusy("save");
    try {
      await onSave({ form, apiKey, provider });
      onClose();
    } catch (err) {
      setMessage({ tone: "error", text: err.message });
    } finally {
      setBusy("");
    }
  }, [apiKey, form, onClose, onSave, provider, validate]);

  const runTest = React.useCallback(async () => {
    if (!form.model.trim()) {
      setMessage({ tone: "error", text: t("llm.modelRequired") });
      return;
    }
    setBusy("test");
    try {
      const result = await onTest(providerPayload(provider, form, apiKey, builtinOverrides));
      setMessage({ tone: result.ok ? "success" : "error", text: result.message });
    } catch (err) {
      setMessage({ tone: "error", text: err.message });
    } finally {
      setBusy("");
    }
  }, [apiKey, builtinOverrides, form, onTest, provider, t]);

  const fetchModels = React.useCallback(async () => {
    if (!form.baseUrl.trim()) {
      setMessage({ tone: "error", text: t("llm.baseUrlRequired") });
      return;
    }
    setBusy("models");
    try {
      const result = await onListModels(providerPayload(provider, form, apiKey, builtinOverrides));
      if (!result.ok || !Array.isArray(result.models) || !result.models.length) {
        setMessage({ tone: "error", text: result.message || t("llm.modelsFetchFailed") });
      } else {
        setModels(result.models);
        setMessage({ tone: "success", text: t("llm.modelsFetched", { count: result.models.length }) });
      }
    } catch (err) {
      setMessage({ tone: "error", text: err.message });
    } finally {
      setBusy("");
    }
  }, [apiKey, builtinOverrides, form, onListModels, provider, t]);

  return {
    form,
    apiKey,
    models,
    message,
    busy,
    isBuiltin,
    isEditing,
    setApiKey,
    update,
    submit,
    runTest,
    fetchModels,
    markIdEdited: () => {
      idEditedRef.current = true;
    },
  };
}
