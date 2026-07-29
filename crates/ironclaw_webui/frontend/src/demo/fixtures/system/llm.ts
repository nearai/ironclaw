// LLM provider fixtures. The `/llm/providers` snapshot is the single source
// of truth the settings inference tab and the first-run onboarding gate read:
// it MUST carry a non-null `active` selection or the whole app redirects to
// /welcome (see src/lib/onboarding-gate.ts).

type LlmProvider = {
  id: string;
  /** Display name — useLlmProviders maps `description` onto `name`. */
  description: string;
  adapter: "open_ai_completions" | "anthropic" | "ollama" | "nearai";
  builtin: boolean;
  api_key_set: boolean;
  api_key_required?: boolean;
  base_url_required?: boolean;
  accepts_api_key?: boolean;
  base_url: string;
  default_model: string;
};

type ActiveSelection = { provider_id: string; model: string } | null;

export const llmProviders: LlmProvider[] = [
  {
    id: "anthropic",
    description: "Anthropic",
    adapter: "anthropic",
    builtin: true,
    api_key_set: true,
    api_key_required: true,
    base_url_required: false,
    base_url: "",
    default_model: "claude-sonnet-4-5",
  },
  {
    id: "nearai",
    description: "NEAR AI",
    adapter: "nearai",
    builtin: true,
    api_key_set: true,
    api_key_required: true,
    base_url_required: false,
    base_url: "",
    default_model: "deepseek-v3.2",
  },
  {
    id: "openai",
    description: "OpenAI",
    adapter: "open_ai_completions",
    builtin: true,
    api_key_set: false,
    api_key_required: true,
    base_url_required: false,
    base_url: "",
    default_model: "gpt-5.2",
  },
  {
    id: "workstation-ollama",
    description: "Workstation Ollama",
    adapter: "ollama",
    builtin: false,
    api_key_set: false,
    base_url: "http://127.0.0.1:11434/v1",
    default_model: "qwen3:32b",
  },
];

export let llmActive: ActiveSelection = {
  provider_id: "anthropic",
  model: "claude-sonnet-4-5",
};

export function llmSnapshot() {
  return { providers: llmProviders, active: llmActive };
}

export function setLlmActive(providerId: string, model: string) {
  llmActive = { provider_id: providerId, model };
}

/** Upsert from the settings dialog: built-in override or custom provider. */
export function upsertLlmProvider(body: Record<string, unknown>) {
  const id = String(body.id || "");
  if (!id) return;
  let provider = llmProviders.find((entry) => entry.id === id);
  if (!provider) {
    provider = {
      id,
      description: String(body.name || id),
      adapter: (body.adapter as LlmProvider["adapter"]) || "open_ai_completions",
      builtin: false,
      api_key_set: false,
      base_url: "",
      default_model: "",
    };
    llmProviders.push(provider);
  }
  if (typeof body.name === "string" && body.name) provider.description = body.name;
  if (typeof body.base_url === "string") provider.base_url = body.base_url;
  if (typeof body.default_model === "string" && body.default_model) {
    provider.default_model = body.default_model;
  }
  if (typeof body.api_key === "string" && body.api_key.trim()) {
    provider.api_key_set = true;
  }
  if (body.set_active === true) {
    setLlmActive(id, String(body.model || provider.default_model || ""));
  }
}

export function deleteLlmProvider(providerId: string) {
  const index = llmProviders.findIndex(
    (entry) => entry.id === providerId && !entry.builtin
  );
  if (index >= 0) llmProviders.splice(index, 1);
  if (llmActive?.provider_id === providerId) {
    llmActive = { provider_id: "anthropic", model: "claude-sonnet-4-5" };
  }
}

const MODELS_BY_ADAPTER: Record<string, string[]> = {
  anthropic: ["claude-opus-4-1", "claude-sonnet-4-5", "claude-haiku-4-5"],
  open_ai_completions: ["gpt-5.2", "gpt-5.2-mini", "o4-mini", "gpt-4.1"],
  ollama: ["qwen3:32b", "llama4:70b", "gemma3:27b", "deepseek-r1:14b"],
  nearai: ["deepseek-v3.2", "qwen3-coder-480b", "llama-4-maverick"],
};

export function modelsForAdapter(adapter: unknown): string[] {
  return MODELS_BY_ADAPTER[String(adapter)] || MODELS_BY_ADAPTER.open_ai_completions;
}
