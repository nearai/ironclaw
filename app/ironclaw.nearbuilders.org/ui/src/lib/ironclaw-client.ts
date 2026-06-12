export interface IronclawSession {
  tenant_id: string;
  user_id: string;
  capabilities: { operator_webui_config: boolean };
}

export interface Thread {
  id: string;
  title?: string;
  createdAt?: string;
  updatedAt?: string;
}

export interface ThreadList {
  data: Thread[];
  meta: {
    total: number;
    hasMore: boolean;
    nextCursor: string | null;
  };
}

export interface TimelineEntry {
  id: string;
  role?: string;
  content?: string;
  createdAt?: string;
}

export interface Timeline {
  data: TimelineEntry[];
  meta: {
    total: number;
    hasMore: boolean;
    nextCursor: string | null;
  };
}

export interface AcceptedResponse {
  outcome: string;
  thread_id: string;
  run_id?: string;
  active_run_id?: string;
  accepted_message_ref: string;
  status: string;
  event_cursor?: number;
}

export interface ChatEvent {
  cursor?: string;
  type:
    | "accepted"
    | "running"
    | "capability_progress"
    | "capability_activity"
    | "capability_display_preview"
    | "gate"
    | "auth_required"
    | "final_reply"
    | "cancelled"
    | "failed"
    | "projection_snapshot"
    | "projection_update"
    | "keep_alive";
  ack?: {
    outcome: string;
    thread_id: string;
    run_id?: string;
    accepted_message_ref: string;
    status: string;
  };
  progress?: { kind: string; message?: string };
  reply?: { text?: string; turn_run_id?: string; generated_at?: string };
  prompt?: { gate_ref?: string; run_id?: string };
}

export type GateResolution = "approved" | "denied" | "credential_provided" | "cancelled";

export interface IronclawConfig {
  baseUrl: string;
  token: string;
}

const STORAGE_KEY = "ironclaw:config";

export function getStoredConfig(): IronclawConfig | null {
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

export function storeConfig(config: IronclawConfig): void {
  sessionStorage.setItem(STORAGE_KEY, JSON.stringify(config));
}

export function clearConfig(): void {
  sessionStorage.removeItem(STORAGE_KEY);
}

export class IronclawClient {
  private baseUrl: string;
  private token: string;

  constructor(baseUrl: string, token: string) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.token = token;
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    const url = `${this.baseUrl}${path}`;
    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.token}`,
    };

    const bodyMethods = new Set(["POST", "PUT", "PATCH"]);
    if (body !== undefined || bodyMethods.has(method.toUpperCase())) {
      headers["Content-Type"] = "application/json";
    }

    const response = await fetch(url, {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });

    if (!response.ok) {
      const errorBody = await response.text().catch(() => "");
      throw new Error(`Ironclaw API error ${response.status}: ${errorBody}`);
    }

    return response.json();
  }

  async getSession(): Promise<IronclawSession> {
    return this.request<IronclawSession>("GET", "/api/webchat/v2/session");
  }

  async listThreads(
    limit?: number,
    cursor?: string,
  ): Promise<ThreadList> {
    const params = new URLSearchParams();
    if (limit !== undefined) params.set("limit", String(limit));
    if (cursor !== undefined) params.set("cursor", cursor);
    const qs = params.toString();
    const raw: any = await this.request(
      "GET",
      `/api/webchat/v2/threads${qs ? `?${qs}` : ""}`,
    );
    return {
      data: (raw.threads ?? []).map((t: any) => ({
        id: t.thread_id,
        title: t.title ?? undefined,
      })),
      meta: {
        total: 0,
        hasMore: raw.next_cursor != null,
        nextCursor: raw.next_cursor ?? null,
      },
    };
  }

  async createThread(): Promise<Thread> {
    const raw: any = await this.request("POST", "/api/webchat/v2/threads");
    const t = raw.thread ?? raw;
    return {
      id: t.thread_id,
      title: t.title ?? undefined,
    };
  }

  async deleteThread(threadId: string): Promise<void> {
    await this.request<void>("DELETE", `/api/webchat/v2/threads/${encodeURIComponent(threadId)}`);
  }

  async sendMessage(
    threadId: string,
    content: string,
    clientActionId?: string,
  ): Promise<AcceptedResponse> {
    const client_action_id = clientActionId ?? crypto.randomUUID();
    return this.request<AcceptedResponse>(
      "POST",
      `/api/webchat/v2/threads/${encodeURIComponent(threadId)}/messages`,
      { client_action_id, content },
    );
  }

  async getTimeline(
    threadId: string,
    limit?: number,
    cursor?: string,
  ): Promise<Timeline> {
    const params = new URLSearchParams();
    if (limit !== undefined) params.set("limit", String(limit));
    if (cursor !== undefined) params.set("cursor", cursor);
    const qs = params.toString();
    const raw: any = await this.request(
      "GET",
      `/api/webchat/v2/threads/${encodeURIComponent(threadId)}/timeline${qs ? `?${qs}` : ""}`,
    );
    return {
      data: (raw.messages ?? []).map((m: any) => ({
        id: m.message_id,
        role: m.kind === "User" ? "user" : m.kind === "System" ? "system" : "assistant",
        content: m.content ?? "",
      })),
      meta: {
        total: 0,
        hasMore: raw.next_cursor != null,
        nextCursor: raw.next_cursor ?? null,
      },
    };
  }

  streamEvents(threadId: string): EventSource {
    const url = `${this.baseUrl}/api/webchat/v2/threads/${encodeURIComponent(threadId)}/events?token=${encodeURIComponent(this.token)}`;
    return new EventSource(url);
  }

  async cancelRun(threadId: string, runId: string): Promise<void> {
    await this.request<void>(
      "POST",
      `/api/webchat/v2/threads/${encodeURIComponent(threadId)}/runs/${encodeURIComponent(runId)}/cancel`,
    );
  }

  async resolveGate(
    threadId: string,
    runId: string,
    gateRef: string,
    resolution: GateResolution,
  ): Promise<void> {
    await this.request<void>(
      "POST",
      `/api/webchat/v2/threads/${encodeURIComponent(threadId)}/runs/${encodeURIComponent(runId)}/gates/${encodeURIComponent(gateRef)}/resolve`,
      { resolution },
    );
  }
}
