import type { UIMessage } from "@tanstack/ai";
import type { StreamChunk } from "@tanstack/ai";

export interface ThreadSession {
  messages: UIMessage[];
  isLoading: boolean;
  error: string | null;
  abortController: AbortController | null;
  listeners: Set<() => void>;
  connectedAt: number;
  runId: string | null;
  pendingApprovals: Array<{ gateRef: string; headline: string }>;
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

async function connectStream(threadId: string, content: string) {
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
    const response = await fetch(`/api/conversation/threads/${encodeURIComponent(threadId)}/chat`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        threadId,
        messages: [{ id: crypto.randomUUID(), role: "user" as const, content }],
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
      current.error = err instanceof Error ? err.message : String(err);
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

  if (chunk.type === "TOOL_CALL_END") {
    for (const msg of session.messages) {
      if (msg.role !== "assistant") continue;
      for (let i = msg.parts.length - 1; i >= 0; i--) {
        const p = msg.parts[i] as any;
        if (p.type !== "tool-call" || p.id !== chunk.toolCallId) continue;
        const next = msg.parts[i + 1] as any;
        if (next?.type === "tool-result" && next.toolCallId === chunk.toolCallId) return;
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
    session.pendingApprovals.push({
      gateRef: String((chunk.value as any)?.approval?.id ?? ""),
      headline: String((chunk.value as any)?.input ?? "Approval required"),
    });
    session.version++;
    notify(threadId);
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
    if (session.messages.length === 0 && !session.isLoading) {
      session.messages = messages;
      session.version++;
      notify(threadId);
    }
  },

  sendMessage(threadId: string, content: string) {
    const session = sessions.get(threadId);
    if (!session || session.isLoading) return;
    session.messages.push({
      id: `pending-${crypto.randomUUID()}`,
      role: "user",
      parts: [{ type: "text", content }],
    });
    session.version++;
    notify(threadId);
    connectStream(threadId, content);
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
