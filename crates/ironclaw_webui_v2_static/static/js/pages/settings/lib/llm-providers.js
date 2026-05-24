export const API_KEY_UNCHANGED = "\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022";

export const ADAPTER_OPTIONS = [
  { value: "open_ai_completions", label: "OpenAI Compatible" },
  { value: "anthropic", label: "Anthropic" },
  { value: "ollama", label: "Ollama" },
  { value: "nearai", label: "NEAR AI" },
];

export function adapterLabel(adapter) {
  return ADAPTER_OPTIONS.find((item) => item.value === adapter)?.label || adapter;
}

export function parseCustomProviders(value) {
  if (Array.isArray(value)) return value;
  if (!value || typeof value !== "string") return [];
  try {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed) ? parsed : [];
  } catch (_) {
    return [];
  }
}

export function parseBuiltinOverrides(value) {
  if (value && typeof value === "object" && !Array.isArray(value)) return value;
  if (!value || typeof value !== "string") return {};
  try {
    const parsed = JSON.parse(value);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
  } catch (_) {
    return {};
  }
}

export function providerEffectiveBaseUrl(provider, overrides) {
  const override = provider.builtin ? overrides[provider.id] || {} : {};
  return override.base_url || provider.env_base_url || provider.base_url || "";
}

export function providerDisplayModel(provider, overrides, activeId, selectedModel) {
  const override = provider.builtin ? overrides[provider.id] || {} : {};
  if (provider.id === activeId) {
    return selectedModel || override.model || provider.env_model || provider.default_model || "";
  }
  return override.model || provider.env_model || provider.default_model || "";
}

export function providerDefaultModel(provider, overrides) {
  const override = provider.builtin ? overrides[provider.id] || {} : {};
  return override.model || provider.env_model || provider.default_model || "";
}

export function isProviderConfigured(provider, overrides) {
  const override = provider.builtin ? overrides[provider.id] || {} : {};
  const needsKey = provider.builtin ? provider.api_key_required !== false : provider.adapter !== "ollama";
  const storedKey = provider.builtin ? override.api_key : provider.api_key;
  const hasDbKey = storedKey === API_KEY_UNCHANGED || (typeof storedKey === "string" && storedKey.length > 0);
  const keyOk = !needsKey || provider.has_api_key === true || hasDbKey;
  if (!keyOk) return false;

  const needsBaseUrl = provider.builtin ? provider.base_url_required === true : true;
  if (!needsBaseUrl) return true;
  return providerEffectiveBaseUrl(provider, overrides).trim().length > 0;
}

export function providerMissingReason(provider, overrides) {
  const override = provider.builtin ? overrides[provider.id] || {} : {};
  const needsKey = provider.builtin ? provider.api_key_required !== false : provider.adapter !== "ollama";
  const storedKey = provider.builtin ? override.api_key : provider.api_key;
  const hasDbKey = storedKey === API_KEY_UNCHANGED || (typeof storedKey === "string" && storedKey.length > 0);
  if (needsKey && provider.has_api_key !== true && !hasDbKey) return "api_key";

  const needsBaseUrl = provider.builtin ? provider.base_url_required === true : true;
  if (needsBaseUrl && !providerEffectiveBaseUrl(provider, overrides).trim()) return "base_url";
  return "ok";
}

export function providerPayload(provider, form, apiKey, overrides) {
  const baseUrl = form.baseUrl.trim();
  const model = form.model.trim();
  const payload = {
    adapter: provider?.builtin ? provider.adapter : form.adapter,
    base_url: baseUrl || provider?.base_url || "",
    provider_id: provider?.id || form.id.trim(),
    provider_type: provider?.builtin ? "builtin" : "custom",
  };
  if (model) payload.model = model;
  if (apiKey.trim()) payload.api_key = apiKey.trim();

  const override = provider?.builtin ? overrides[provider.id] || {} : {};
  if (!payload.api_key && override.api_key === API_KEY_UNCHANGED) {
    payload.api_key = undefined;
  }
  return payload;
}

export function providerIdFromName(name) {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9_]+/g, "-")
    .replace(/^-|-$/g, "");
}

export function isValidProviderId(id) {
  return /^[a-z0-9_-]+$/.test(id);
}
