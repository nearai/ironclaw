import { createFileRoute, redirect } from "@tanstack/react-router";
import { Loader2, MessageSquare, Plus, Send, Terminal, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { useApiClient, sessionQueryOptions } from "@/app";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
type StreamEvent = {
  type: string;
  ack?: Record<string, unknown>;
  progress?: Record<string, unknown>;
  reply?: Record<string, unknown>;
  [key: string]: unknown;
};

type ThreadItem = {
  threadId: string;
  title?: string;
};

type ChatMessage = {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: string;
};

export const Route = createFileRoute("/_layout/")({
  beforeLoad: async ({ context, location }) => {
    const { queryClient, authClient, session } = context;
    const current = await queryClient.ensureQueryData(
      sessionQueryOptions(authClient, session),
    );
    if (!current?.user) {
      throw redirect({ to: "/login", search: { redirect: location.pathname } });
    }
    return { session: current };
  },
  component: ChatPage,
});

function ChatPage() {
  const apiClient = useApiClient();
  const [threads, setThreads] = useState<ThreadItem[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Map<string, ChatMessage[]>>(new Map());
  const [inputText, setInputText] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [streamingEvent, setStreamingEvent] = useState<StreamEvent | null>(null);
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [sseError, setSseError] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const streamCleanupRef = useRef<() => void>(null);

  const loadThreads = useCallback(async () => {
    try {
      const result = await apiClient.ironclaw.threads.list({ limit: 50 });
      setThreads(result.data);
    } catch (err) {
      console.error("Failed to load threads:", err);
    }
  }, [apiClient]);

  useEffect(() => {
    loadThreads();
  }, [loadThreads]);

  const closeStream = useCallback(() => {
    streamCleanupRef.current?.();
    streamCleanupRef.current = null;
  }, []);

  const openThread = useCallback(async (threadId: string) => {
    closeStream();

    setActiveThreadId(threadId);
    setStreamingEvent(null);
    setCurrentRunId(null);
    setSseError(false);

    try {
      const timeline = await apiClient.ironclaw.threads.getTimeline({ id: threadId, limit: 100 });
      const msgs: ChatMessage[] = timeline.data.map((entry) => ({
        id: entry.messageId,
        role: entry.role === "user" ? "user" : "assistant",
        content: entry.content ?? "",
        timestamp: entry.createdAt ?? new Date().toISOString(),
      }));
      setMessages((prev) => new Map(prev).set(threadId, msgs));
    } catch (err) {
      console.error("Failed to load timeline:", err);
    }

    (async () => {
      try {
        const eventStream = await apiClient.ironclaw.threads.streamEvents({ id: threadId });
        streamCleanupRef.current = () => { eventStream.return?.(); };

        for await (const data of eventStream) {
          setSseError(false);
          setStreamingEvent(data);

          if (data.type === "accepted" && data.ack?.runId) {
            setCurrentRunId(data.ack.runId);
          }

          if (data.type === "final_reply" && (data.reply?.text ?? data.reply?.turnRunId)) {
            setMessages((prev) => {
              const existing = prev.get(threadId) ?? [];
              return new Map(prev).set(threadId, [
                ...existing,
                {
                  id: `reply-${crypto.randomUUID()}`,
                  role: "assistant",
                  content: data.reply?.text ?? "",
                  timestamp: new Date().toISOString(),
                },
              ]);
            });
            setStreamingEvent(null);
            setCurrentRunId(null);
          }

          if (data.type === "cancelled" || data.type === "failed") {
            setCurrentRunId(null);
          }
        }
      } catch {
        setSseError(true);
      }
    })();
  }, [apiClient]);

  useEffect(() => {
    return () => {
      closeStream();
    };
  }, [closeStream]);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, activeThreadId, streamingEvent]);

  const createThread = useCallback(async () => {
    try {
      const thread = await apiClient.ironclaw.threads.create();
      setThreads((prev) => [thread, ...prev]);
      setActiveThreadId(thread.threadId);
      setMessages((prev) => new Map(prev).set(thread.threadId, []));
    } catch {
      toast.error("Failed to create thread");
    }
  }, [apiClient]);

  const deleteThread = useCallback(
    async (threadId: string) => {
      try {
        await apiClient.ironclaw.threads.delete({ id: threadId });
        setThreads((prev) => prev.filter((t) => t.threadId !== threadId));
        if (activeThreadId === threadId) {
          setActiveThreadId(null);
        }
      } catch {
        toast.error("Failed to delete thread");
      }
    },
    [apiClient, activeThreadId],
  );

  const sendMessage = useCallback(async () => {
    if (!activeThreadId || !inputText.trim() || isSending) return;

    const content = inputText.trim();
    setInputText("");
    setIsSending(true);

    setMessages((prev) => {
      const existing = prev.get(activeThreadId) ?? [];
      return new Map(prev).set(activeThreadId, [
        ...existing,
        {
          id: `user-${crypto.randomUUID()}`,
          role: "user",
          content,
          timestamp: new Date().toISOString(),
        },
      ]);
    });

    try {
      await apiClient.ironclaw.threads.sendMessage({ id: activeThreadId, content });
    } catch {
      toast.error("Failed to send message");
    } finally {
      setIsSending(false);
    }
  }, [activeThreadId, inputText, isSending, apiClient]);

  const currentMessages = activeThreadId ? (messages.get(activeThreadId) ?? []) : [];

  return (
    <div className="flex h-full w-full">
      <div className="flex h-full w-72 shrink-0 flex-col border-r border-border bg-card">
        <div className="flex items-center justify-between border-b border-border px-3 py-2.5">
          <div className="flex items-center gap-1.5">
            <span className="h-1.5 w-1.5 rounded-full bg-green-500" />
            <span className="text-xs font-medium text-muted-foreground">Threads</span>
          </div>
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={createThread}>
            <Plus size={14} />
          </Button>
        </div>
        <ScrollArea className="flex-1">
          <div className="space-y-0.5 p-2">
            {threads.length === 0 && (
              <p className="px-2 py-4 text-center text-xs text-muted-foreground">
                No threads yet. Create one to start chatting.
              </p>
            )}
            {threads.map((thread) => (
              <div
                key={thread.threadId}
                role="button"
                tabIndex={0}
                onClick={() => openThread(thread.threadId)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    openThread(thread.threadId);
                  }
                }}
                className={`group flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm transition-colors cursor-pointer ${
                  activeThreadId === thread.threadId
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-muted"
                }`}
              >
                <MessageSquare size={14} className="shrink-0" />
                <span className="flex-1 truncate">
                  {thread.title ?? `Thread ${thread.threadId.slice(0, 8)}`}
                </span>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    deleteThread(thread.threadId);
                  }}
                  className="shrink-0 opacity-0 group-hover:opacity-100 transition-opacity"
                >
                  <Trash2 size={12} className="text-muted-foreground hover:text-destructive" />
                </button>
              </div>
            ))}
          </div>
        </ScrollArea>
      </div>

      <div className="flex flex-1 flex-col min-w-0">
        {activeThreadId ? (
          <>
            <ScrollArea className="flex-1 p-4">
              <div className="mx-auto max-w-3xl space-y-4">
                {currentMessages.map((msg) => (
                  <div
                    key={msg.id}
                    className={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}
                  >
                    <div
                      className={`max-w-[80%] rounded-xl px-4 py-2.5 text-sm ${
                        msg.role === "user"
                          ? "bg-primary text-primary-foreground"
                          : "bg-muted text-foreground"
                      }`}
                    >
                      <p className="whitespace-pre-wrap">{msg.content}</p>
                    </div>
                  </div>
                ))}
                {currentRunId && streamingEvent?.type !== "running" && streamingEvent?.type !== "gate" && (
                  <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    Message received, waiting...
                  </div>
                )}
                {streamingEvent && streamingEvent.type === "running" && (
                  <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    {String(streamingEvent.progress?.message ?? "") || "Thinking..."}
                  </div>
                )}
                {streamingEvent?.type === "gate" && (
                  <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-4 py-3 text-xs text-amber-600">
                    Gate requires resolution — check your ironclaw console
                  </div>
                )}
                {streamingEvent?.type === "cancelled" && (
                  <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-xs text-destructive">
                    Run cancelled
                  </div>
                )}
                {streamingEvent?.type === "failed" && (
                  <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-xs text-destructive">
                    Run failed
                  </div>
                )}
                {sseError && (
                  <div className="flex items-center gap-2 rounded-lg border border-amber-500/30 bg-amber-500/5 px-4 py-3 text-xs text-amber-600">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    Reconnecting to event stream...
                  </div>
                )}
                <div ref={messagesEndRef} />
              </div>
            </ScrollArea>

            <div className="border-t border-border p-4">
              <div className="mx-auto flex max-w-3xl items-center gap-2">
                <Input
                  value={inputText}
                  onChange={(e) => setInputText(e.target.value)}
                  placeholder="Type a message..."
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !e.shiftKey) {
                      e.preventDefault();
                      sendMessage();
                    }
                  }}
                  disabled={isSending}
                />
                <Button
                  size="icon"
                  onClick={sendMessage}
                  disabled={!inputText.trim() || isSending}
                >
                  <Send size={14} />
                </Button>
              </div>
            </div>
          </>
        ) : (
          <div className="flex h-full items-center justify-center">
            <div className="text-center">
              <Terminal className="mx-auto h-8 w-8 text-muted-foreground" />
              <p className="mt-2 text-sm text-muted-foreground">
                Select a thread or create a new one
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
