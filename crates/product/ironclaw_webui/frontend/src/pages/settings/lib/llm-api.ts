import { apiFetch, clientActionId, type ApiRecord } from "../../../lib/api";

export interface UserModelPolicyResponse extends ApiRecord {
  provider_id: string;
  workspace_default: string;
  allowed_models: string[];
  model_entries: ApiRecord[];
}

export interface LlmProvidersResponse extends ApiRecord {
  providers: LlmProviderResponse[];
  active: (ApiRecord & { provider_id: string; model?: string | null }) | null;
  user_model_policy: UserModelPolicyResponse | null;
}

interface LlmProviderResponse extends ApiRecord {
  id: string;
  description: string;
  adapter: string;
  default_model: string;
  base_url?: string | null;
  builtin: boolean;
  active: boolean;
  active_model?: string | null;
  api_key_required: boolean;
  accepts_api_key: boolean;
  api_key_set: boolean;
  can_list_models: boolean;
}

export interface UserModelCatalogResponse extends ApiRecord {
  models: string[];
  model_entries?: ApiRecord[];
  workspace_default?: string | null;
  selection_enabled?: boolean;
}

export interface UserModelPreferenceResponse extends ApiRecord {
  model?: string | null;
}

function invalidProviders(): never {
  throw new TypeError("invalid LLM providers response");
}

function record(value: unknown): ApiRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as ApiRecord
    : invalidProviders();
}

function records(value: unknown): ApiRecord[] {
  return Array.isArray(value) && value.every((entry) => record(entry))
    ? value
    : invalidProviders();
}

function decodeProvider(value: unknown): LlmProviderResponse {
  const provider = record(value);
  if (
    typeof provider.id !== "string" ||
    typeof provider.description !== "string" ||
    typeof provider.adapter !== "string" ||
    typeof provider.default_model !== "string" ||
    (provider.base_url !== undefined &&
      provider.base_url !== null &&
      typeof provider.base_url !== "string") ||
    typeof provider.builtin !== "boolean" ||
    typeof provider.active !== "boolean" ||
    (provider.active_model !== undefined &&
      provider.active_model !== null &&
      typeof provider.active_model !== "string") ||
    typeof provider.api_key_required !== "boolean" ||
    typeof provider.accepts_api_key !== "boolean" ||
    typeof provider.api_key_set !== "boolean" ||
    typeof provider.can_list_models !== "boolean"
  ) {
    return invalidProviders();
  }
  return provider as LlmProviderResponse;
}

function decodeActive(value: unknown): LlmProvidersResponse["active"] {
  if (value === undefined || value === null) return null;
  const active = record(value);
  if (
    typeof active.provider_id !== "string" ||
    (active.model !== undefined &&
      active.model !== null &&
      typeof active.model !== "string")
  ) {
    return invalidProviders();
  }
  return active as LlmProvidersResponse["active"];
}

function decodeUserModelPolicy(value: unknown): UserModelPolicyResponse | null {
  if (value === undefined || value === null) return null;
  const policy = record(value);
  if (
    typeof policy.provider_id !== "string" ||
    typeof policy.workspace_default !== "string" ||
    !Array.isArray(policy.allowed_models) ||
    !policy.allowed_models.every((model) => typeof model === "string")
  ) {
    return invalidProviders();
  }
  return {
    ...policy,
    allowed_models: policy.allowed_models,
    model_entries: records(policy.model_entries),
  } as UserModelPolicyResponse;
}

function decodeLlmProvidersResponse(value: unknown): LlmProvidersResponse {
  const response = record(value);
  if (!Array.isArray(response.providers)) return invalidProviders();
  return {
    ...response,
    providers: response.providers.map(decodeProvider),
    active: decodeActive(response.active),
    user_model_policy: decodeUserModelPolicy(response.user_model_policy),
  };
}

// LLM provider configuration — v2 native endpoints. The snapshot is the single
// source of truth: a unified provider list (built-in + operator-defined) plus
// the active selection. API-key values are write-only; the snapshot only ever
// reports `api_key_set`.
export async function fetchLlmProviders(): Promise<LlmProvidersResponse> {
  return apiFetch(
    "/api/webchat/v2/llm/providers",
    {},
    decodeLlmProvidersResponse,
  );
}

export function fetchUserModelCatalog(): Promise<UserModelCatalogResponse> {
  return apiFetch<UserModelCatalogResponse>("/api/webchat/v2/llm/models");
}

export function fetchUserModelPreference(): Promise<UserModelPreferenceResponse> {
  return apiFetch<UserModelPreferenceResponse>(
    "/api/webchat/v2/llm/model-preference",
  );
}

export function setUserModelPolicy(payload): Promise<UserModelCatalogResponse> {
  return apiFetch<UserModelCatalogResponse>("/api/webchat/v2/llm/model-policy", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function setUserModelPreference(
  model,
): Promise<UserModelPreferenceResponse> {
  return apiFetch<UserModelPreferenceResponse>(
    "/api/webchat/v2/llm/model-preference",
    {
      method: "PUT",
      body: JSON.stringify({ model }),
    },
  );
}

export function upsertLlmProvider(payload) {
  const { clientActionId: callerClientActionId, ...request } = payload;
  return apiFetch("/api/webchat/v2/llm/providers", {
    method: "POST",
    body: JSON.stringify({
      ...request,
      client_action_id:
        request.client_action_id || callerClientActionId || clientActionId(),
    }),
  });
}

export function deleteLlmProvider(providerId) {
  return apiFetch(
    `/api/webchat/v2/llm/providers/${encodeURIComponent(providerId)}/delete`,
    { method: "POST" },
  );
}

export function setActiveLlm(payload) {
  return apiFetch("/api/webchat/v2/llm/active", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function testLlmProviderConnection(payload) {
  return apiFetch("/api/webchat/v2/llm/test-connection", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function listLlmProviderModels(payload) {
  return apiFetch("/api/webchat/v2/llm/list-models", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

// Begin NEAR AI browser login. Returns { auth_url } to open; a background task
// stores the session token and makes NEAR AI active once the user authorizes.
export function startNearaiLogin(payload): Promise<ApiRecord & { auth_url: string }> {
  return apiFetch<ApiRecord & { auth_url: string }>(
    "/api/webchat/v2/llm/nearai/login",
    { method: "POST", body: JSON.stringify(payload) },
  );
}

// Complete a NEAR AI wallet (NEP-413) login. `payload` carries the browser
// wallet's signed message; the backend relays it to NEAR AI, stores the session
// token, and makes NEAR AI active. Returns { active }.
export function completeNearaiWalletLogin(payload) {
  return apiFetch("/api/webchat/v2/llm/nearai/wallet", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

// Begin an OpenAI Codex (ChatGPT subscription) device-code login. Returns
// { user_code, verification_uri } to display; a background task polls for
// authorization, stores the tokens, and makes Codex active once authorized.
export function startCodexLogin(): Promise<
  ApiRecord & { user_code: string; verification_uri: string }
> {
  return apiFetch<ApiRecord & { user_code: string; verification_uri: string }>(
    "/api/webchat/v2/llm/codex/login",
    { method: "POST" },
  );
}
