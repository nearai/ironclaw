import type { UIMessage } from "@tanstack/ai";
import type { StreamChunk } from "@tanstack/ai";

export interface ApprovalAction {
  label?: string;
  method?: string;
}

export interface ApprovalScope {
  label?: string;
  reusable?: boolean;
}

export interface ApprovalDestination {
  label?: string;
  url?: string;
  domain?: string;
}

export interface ApprovalDetail {
  label?: string;
  value?: string;
}

export interface PendingApproval {
  gateRef: string;
  headline: string;
  toolName?: string;
  description?: string;
  allowAlways?: boolean;
  action?: ApprovalAction;
  scope?: ApprovalScope;
  destination?: ApprovalDestination;
  details?: ApprovalDetail[];
}

export interface AuthGate {
  runId: string;
  gateRef: string;
  challengeKind: string;
  provider?: string;
  accountLabel?: string;
  authorizationUrl?: string;
  expiresAt?: string;
  headline?: string;
  body?: string;
}

export interface ThreadSession {
  messages: UIMessage[];
  isLoading: boolean;
  error: string | null;
  abortController: AbortController | null;
  listeners: Set<() => void>;
  connectedAt: number;
  runId: string | null;
  pendingApprovals: PendingApproval[];
  authGates: AuthGate[];
  lastRunErrorData: unknown;
  version: number;
}

const sessions = new Map<string, ThreadSession>();

function notify(threadId: string) {
  const session = sessions.get(threadId);
  if (session) {
    for (const listener of session.listeners) {
      try {
        listener();
      } catch {}
    }
  }
}

function stagedToBridgeFormat(a: any) {
  return {
    mimeType: a.mimeType,
    filename: a.filename ?? undefined,
    dataBase64: a.dataBase64,
  };
}

async function connectStream(threadId: string, content: string, attachments?: any[]) {
  const session = sessions.get(threadId);
  if (!session) return;

  session.isLoading = true;
  session.error = null;
  session.version++;
  notify(threadId);

  const ac = new AbortController();
  session.abortController = ac;
  session.pendingApprovals = [];

  try {
    const forwardedAttachments = attachments?.map(stagedToBridgeFormat);
    const response = await fetch(`/api/conversation/threads/${encodeURIComponent(threadId)}/chat`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        threadId,
        messages: [{ id: crypto.randomUUID(), role: "user" as const, content }],
        forwardedProps: forwardedAttachments ? { attachments: forwardedAttachments } : undefined,
      }),
      signal: ac.signal,
    });

    if (!response.ok) {
      const text = await response.text().catch(() => "");
      throw new Error(`Chat error ${response.status}: ${text}`);
    }

    if (!response.body) {
      throw new Error("No response body");
    }

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";

    while (true) {
      const current = sessions.get(threadId);
      if (!current || current.abortController !== ac) return;

      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const parts = buffer.split("\n");
      buffer = parts.pop() || "";

      for (const line of parts) {
        const trimmed = line.trim();
        if (!trimmed.startsWith("data: ")) continue;
        try {
          const chunk: StreamChunk & { name?: string; value?: unknown; runId?: string } =
            JSON.parse(trimmed.slice(6));
          handleChunk(threadId, chunk);
        } catch {}
      }
    }
  } catch (err: unknown) {
    if (err instanceof Error && err.name === "AbortError") return;
    const current = sessions.get(threadId);
    if (current && current.abortController === ac) {
      const message = err instanceof Error ? err.message : String(err);
      current.error = message;
      current.messages.push({
        id: `error-${Date.now()}`,
        role: "system",
        parts: [{ type: "text", content: message }],
      });
    }
  } finally {
    const current = sessions.get(threadId);
    if (current && current.abortController === ac) {
      current.isLoading = false;
      current.abortController = null;
      current.version++;
    }
    notify(threadId);
  }
}

function handleChunk(threadId: string, chunk: any) {
  const session = sessions.get(threadId);
  if (!session) return;

  if (chunk.type === "RUN_STARTED") {
    session.runId = chunk.runId ?? null;
    session.version++;
    notify(threadId);
    return;
  }

  if (chunk.type === "RUN_FINISHED" || chunk.type === "RUN_ERROR") {
    session.runId = null;
    session.pendingApprovals = [];
    session.authGates = [];
    for (const msg of session.messages) {
      if (msg.role !== "assistant") continue;
      for (let i = msg.parts.length - 1; i >= 0; i--) {
        const p = msg.parts[i] as any;
        if (p.type !== "tool-call") continue;
        const next = msg.parts[i + 1] as any;
        if (next?.type === "tool-result" && next.toolCallId === p.id) continue;
        msg.parts.splice(i + 1, 0, {
          type: "tool-result",
          toolCallId: p.id,
          content: "",
          state: chunk.type === "RUN_ERROR" ? "error" : "complete",
        });
      }
    }
    if (chunk.type === "RUN_ERROR" && chunk.message) {
      const errorParts: any[] = [{ type: "text", content: chunk.message }];
      const errorData = chunk.details || session.lastRunErrorData;
      if (errorData) {
        errorParts.push({ type: "error-data", content: errorData });
      }
      session.messages.push({
        id: `error-${Date.now()}`,
        role: "system",
        parts: errorParts,
      });
    }
    session.lastRunErrorData = null;
    session.version++;
    notify(threadId);
    return;
  }

  if (chunk.type === "TEXT_MESSAGE_START") {
    const existing = session.messages.find(
      (m) => m.id === chunk.messageId && m.role === "assistant",
    );
    if (!existing) {
      session.messages.push({
        id: chunk.messageId,
        role: "assistant",
        parts: [],
      });
      session.version++;
    }
    notify(threadId);
    return;
  }

  if (chunk.type === "TEXT_MESSAGE_CONTENT") {
    const msg = session.messages.find((m) => m.id === chunk.messageId);
    if (msg) {
      const lastPart = msg.parts[msg.parts.length - 1];
      if (lastPart?.type === "text") {
        lastPart.content += chunk.delta ?? "";
      } else {
        msg.parts.push({ type: "text", content: chunk.delta ?? "" });
      }
      session.version++;
      notify(threadId);
    }
    return;
  }

  if (chunk.type === "TEXT_MESSAGE_END") {
    return;
  }

  if (chunk.type === "TOOL_CALL_START") {
    let msg = session.messages.find(
      (m) => m.id === chunk.parentMessageId && m.role === "assistant",
    );
    if (!msg && chunk.parentMessageId) {
      msg = {
        id: chunk.parentMessageId,
        role: "assistant",
        parts: [],
      };
      session.messages.push(msg);
    }
    if (msg) {
      const alreadyExists = msg.parts.some(
        (p: any) => p.type === "tool-call" && p.id === chunk.toolCallId,
      );
      if (alreadyExists) return;
      msg.parts.push({
        type: "tool-call",
        id: chunk.toolCallId,
        name: chunk.toolCallName ?? chunk.toolName ?? "",
        arguments: chunk.args ?? "{}",
        state: "input-complete",
      });
      session.version++;
      notify(threadId);
    }
    return;
  }

  if (chunk.type === "TOOL_CALL_ARGS") {
    for (const msg of session.messages) {
      if (msg.role !== "assistant") continue;
      for (const p of msg.parts as any[]) {
        if (p.type === "tool-call" && p.id === chunk.toolCallId) {
          p.arguments = chunk.args ?? chunk.delta ?? p.arguments;
          session.version++;
          notify(threadId);
          return;
        }
      }
    }
    return;
  }

  if (chunk.type === "TOOL_CALL_END") {
    for (const msg of session.messages) {
      if (msg.role !== "assistant") continue;
      for (let i = msg.parts.length - 1; i >= 0; i--) {
        const p = msg.parts[i] as any;
        if (p.type !== "tool-call" || p.id !== chunk.toolCallId) continue;
        const next = msg.parts[i + 1] as any;
        if (next?.type === "tool-result" && next.toolCallId === chunk.toolCallId) return;
        p.state = chunk.state === "error" ? "input-complete" : "complete";
        msg.parts.splice(i + 1, 0, {
          type: "tool-result",
          toolCallId: chunk.toolCallId,
          content: chunk.result ?? "",
          state: chunk.state === "error" ? "error" : "complete",
        });
        session.version++;
        notify(threadId);
        return;
      }
    }
    return;
  }

  if (chunk.type === "CUSTOM" && chunk.name === "approval-requested") {
    const val = (chunk.value as any) ?? {};
    const approval = val.approval ?? {};
    session.pendingApprovals.push({
      gateRef: String(approval.id ?? val.toolCallId ?? ""),
      headline: String(val.input ?? "Approval required"),
      toolName: approval.toolName ?? val.toolName ?? undefined,
      description: approval.description ?? undefined,
      allowAlways: approval.allowAlways === true,
      action: approval.action ?? undefined,
      scope: approval.scope ?? undefined,
      destination: approval.destination ?? undefined,
      details: approval.details ?? undefined,
    });
    session.version++;
    notify(threadId);
    return;
  }

  if (chunk.type === "CUSTOM" && chunk.name === "ironclaw.auth-required") {
    const authPrompt = (chunk.value as any) ?? {};
    session.authGates.push({
      runId: String(authPrompt.runId ?? authPrompt.turnRunId ?? ""),
      gateRef: String(authPrompt.authRequestRef ?? authPrompt.auth_request_ref ?? ""),
      challengeKind: String(authPrompt.challengeKind ?? "other"),
      provider: authPrompt.provider ?? undefined,
      accountLabel: authPrompt.accountLabel ?? undefined,
      authorizationUrl: authPrompt.authorizationUrl ?? undefined,
      expiresAt: authPrompt.expiresAt ?? undefined,
      headline: authPrompt.headline ?? undefined,
      body: authPrompt.body ?? undefined,
    });
    session.version++;
    notify(threadId);
    return;
  }

  if (chunk.type === "CUSTOM" && chunk.name === "ironclaw.skill-activation") {
    const val = (chunk.value as any) ?? {};
    const skillNames: string[] = val.skillNames ?? [];
    const feedback: string[] = val.feedback ?? [];
    const content = [...skillNames.map((n) => `Skill activated: ${n}`), ...feedback]
      .filter(Boolean)
      .join("\n");
    if (content) {
      session.messages.push({
        id: `skill-${val.id ?? Date.now()}`,
        role: "system",
        parts: [{ type: "text", content }],
      });
      session.version++;
      notify(threadId);
    }
    return;
  }

  if (chunk.type === "CUSTOM" && chunk.name === "ironclaw.thinking") {
    const val = (chunk.value as any) ?? {};
    const body = String(val.body ?? "");
    if (body) {
      const lastAssistant = [...session.messages]
        .reverse()
        .find((m) => m.role === "assistant");
      if (lastAssistant) {
        lastAssistant.parts.push({ type: "thinking", content: body });
        session.version++;
        notify(threadId);
      }
    }
    return;
  }

  if (chunk.type === "CUSTOM" && chunk.name === "ironclaw.failed") {
    session.lastRunErrorData = (chunk.value as any)?.details ?? chunk.value;
    return;
  }
}

export const threadChatManager = {
  getOrCreate(threadId: string): ThreadSession {
    if (sessions.has(threadId)) {
      return sessions.get(threadId)!;
    }
    const session: ThreadSession = {
      messages: [],
      isLoading: false,
      error: null,
      abortController: null,
      listeners: new Set(),
      connectedAt: Date.now(),
      runId: null,
      pendingApprovals: [],
      authGates: [],
      lastRunErrorData: null,
      version: 0,
    };
    sessions.set(threadId, session);
    return session;
  },

  get(threadId: string): ThreadSession | undefined {
    return sessions.get(threadId);
  },

  subscribe(threadId: string, listener: () => void): () => void {
    const session = this.getOrCreate(threadId);
    session.listeners.add(listener);
    return () => {
      session.listeners.delete(listener);
    };
  },

  hydrate(threadId: string, messages: UIMessage[]) {
    const session = this.getOrCreate(threadId);
    const shouldOverwrite = !session.isLoading || messages.length > session.messages.length;
    if (shouldOverwrite && messages.length > 0) {
      session.messages = messages;
      session.version++;
      notify(threadId);
    }
  },

  sendMessage(threadId: string, content: string, attachments?: any[]) {
    const session = sessions.get(threadId);
    if (!session || session.isLoading) return;
    const parts: any[] = [{ type: "text", content }];
    if (attachments?.length) {
      for (const a of attachments) {
        if (a.kind === "image") {
          parts.push({
            type: "tool-call",
            id: `att-${crypto.randomUUID()}`,
            name: "attachment",
            arguments: JSON.stringify({ filename: a.filename, mimeType: a.mimeType }),
            state: "input-complete",
          });
          parts.push({
            type: "tool-result",
            toolCallId: (parts[parts.length - 1] as any).id,
            state: "complete",
            content: JSON.stringify({
              kind: "image",
              threadId,
              messageId: `pending-${crypto.randomUUID()}`,
              attachmentId: a.id,
              mimeType: a.mimeType,
              filename: a.filename,
              inlineBase64: a.dataBase64,
            }),
          });
        }
      }
    }
    session.messages.push({
      id: `pending-${crypto.randomUUID()}`,
      role: "user",
      parts,
    });
    session.version++;
    notify(threadId);
    connectStream(threadId, content, attachments);
  },

  stop(threadId: string) {
    const session = sessions.get(threadId);
    if (session && session.abortController) {
      session.abortController.abort();
      session.isLoading = false;
      session.abortController = null;
      session.version++;
      notify(threadId);
    }
  },

  destroy(threadId: string) {
    this.stop(threadId);
    sessions.delete(threadId);
  },

  destroyAll() {
    for (const [id] of sessions) {
      this.destroy(id);
    }
  },
};
