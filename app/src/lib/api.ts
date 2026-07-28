import type {
  Automation,
  Session,
  ThreadRecord,
  TimelineResponse,
  ToolSetting
} from "@/types";

const V2 = "/api/webchat/v2";

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly payload?: unknown
  ) {
    super(message);
    this.name = "ApiError";
  }
}

function normalizeOrigin(origin: string): string {
  return origin.trim().replace(/\/+$/, "");
}

function readableError(payload: unknown, statusText: string): string {
  if (payload && typeof payload === "object") {
    const body = payload as Record<string, unknown>;
    const code = body.validation_code ?? body.kind ?? body.error;
    if (typeof code === "string") {
      return code.replace(/[_-]+/g, " ").replace(/^\w/, (value) => value.toUpperCase());
    }
  }
  return statusText || "Request failed";
}

export class IronClawApi {
  readonly origin: string;

  constructor(origin: string, private readonly token: string) {
    this.origin = normalizeOrigin(origin);
  }

  async request<T>(path: string, init: RequestInit = {}): Promise<T> {
    const headers = new Headers(init.headers);
    headers.set("Accept", "application/json");
    if (this.token) headers.set("Authorization", `Bearer ${this.token}`);
    if (init.body && !headers.has("Content-Type")) {
      headers.set("Content-Type", "application/json");
    }
    const response = await fetch(`${this.origin}${path}`, { ...init, headers });
    const contentType = response.headers.get("content-type") ?? "";
    const payload: unknown = contentType.includes("application/json")
      ? await response.json().catch(() => undefined)
      : await response.text().catch(() => "");
    if (!response.ok) {
      throw new ApiError(readableError(payload, response.statusText), response.status, payload);
    }
    if (!contentType.includes("application/json")) {
      throw new ApiError(
        `This deployment does not expose the IronClaw mobile API (${this.origin})`,
        response.status,
        undefined
      );
    }
    return payload as T;
  }

  session() {
    return this.request<Session>(`${V2}/session`);
  }

  listThreads(limit = 100) {
    return this.request<{ threads: ThreadRecord[]; next_cursor?: string }>(
      `${V2}/threads?limit=${limit}`
    );
  }

  createThread(clientActionId: string) {
    return this.request<{ thread: ThreadRecord }>(`${V2}/threads`, {
      method: "POST",
      body: JSON.stringify({ client_action_id: clientActionId })
    });
  }

  deleteThread(threadId: string) {
    return this.request(`${V2}/threads/${encodeURIComponent(threadId)}`, { method: "DELETE" });
  }

  timeline(threadId: string, limit = 100, cursor?: string) {
    const params = new URLSearchParams({ limit: String(limit) });
    if (cursor) params.set("cursor", cursor);
    return this.request<TimelineResponse>(
      `${V2}/threads/${encodeURIComponent(threadId)}/timeline?${params}`
    );
  }

  sendMessage(
    threadId: string,
    content: string,
    actionId: string,
    attachments: Array<{ mime_type: string; filename: string; data_base64: string }> = []
  ) {
    return this.request<Record<string, unknown>>(
      `${V2}/threads/${encodeURIComponent(threadId)}/messages`,
      {
        method: "POST",
        body: JSON.stringify({ content, client_action_id: actionId, ...(attachments.length ? { attachments } : {}) })
      }
    );
  }

  cancelRun(threadId: string, runId: string) {
    return this.request(`${V2}/threads/${encodeURIComponent(threadId)}/runs/${encodeURIComponent(runId)}/cancel`, { method: "POST" });
  }

  retryRun(threadId: string, runId: string, actionId: string) {
    return this.request(`${V2}/threads/${encodeURIComponent(threadId)}/runs/${encodeURIComponent(runId)}/retry`, {
      method: "POST",
      body: JSON.stringify({ client_action_id: actionId })
    });
  }

  listAutomations() {
    return this.request<{ automations: Automation[]; scheduler_enabled: boolean }>(
      `${V2}/automations?limit=100&run_limit=5&include_completed=true`
    );
  }

  mutateAutomation(automationId: string, action: "pause" | "resume") {
    return this.request(`${V2}/automations/${encodeURIComponent(automationId)}/${action}`, {
      method: "POST"
    });
  }

  renameAutomation(automationId: string, name: string) {
    return this.request(`${V2}/automations/${encodeURIComponent(automationId)}`, {
      method: "POST",
      body: JSON.stringify({ name })
    });
  }

  deleteAutomation(automationId: string) {
    return this.request(`${V2}/automations/${encodeURIComponent(automationId)}`, {
      method: "DELETE"
    });
  }

  async toolSettings() {
    const response = await this.request<{ entries?: ToolSetting[] }>(`${V2}/settings/tools`);
    return response.entries ?? [];
  }

  setGlobalAutoApprove(enabled: boolean) {
    return this.request(`${V2}/settings/tools`, {
      method: "POST",
      body: JSON.stringify({ enabled })
    });
  }

  setToolPermission(
    capabilityId: string,
    state: "ask" | "always_allow" | "always_deny"
  ) {
    return this.request(`${V2}/settings/tools/${encodeURIComponent(capabilityId)}`, {
      method: "POST",
      body: JSON.stringify({ state })
    });
  }

  authProviders() {
    return this.request<{ providers: string[] }>("/auth/providers");
  }

  exchangeLoginTicket(ticket: string) {
    return this.request<{ token: string }>("/auth/session/exchange", {
      method: "POST",
      body: JSON.stringify({ ticket })
    });
  }
}

export function providerLoginUrl(origin: string, provider: string, returnUrl: string): string {
  const url = new URL(`/auth/login/${encodeURIComponent(provider)}`, normalizeOrigin(origin));
  url.searchParams.set("redirect_after", returnUrl);
  return url.toString();
}
