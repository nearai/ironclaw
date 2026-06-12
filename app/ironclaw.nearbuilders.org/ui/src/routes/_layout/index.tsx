import { createFileRoute } from "@tanstack/react-router";
import { Loader2, MessageSquare, Plus, Send, Terminal, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  type ChatEvent,
  type IronclawConfig,
  type IronclawSession,
  type Thread,
  type ThreadList,
  type Timeline,
  IronclawClient,
  clearConfig,
  getStoredConfig,
  storeConfig,
} from "@/lib/ironclaw-client";

type ViewState =
  | { type: "setup" }
  | { type: "connecting"; config: IronclawConfig }
  | { type: "connected"; client: IronclawClient; session: IronclawSession }
  | { type: "error"; message: string };

type ChatMessage = {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: string;
};

export const Route = createFileRoute("/_layout/")({
  component: ChatPage,
});

function ChatPage() {
  const [viewState, setViewState] = useState<ViewState>(() => {
    const config = getStoredConfig();
    if (config) return { type: "connecting", config };
    return { type: "setup" };
  });

  const [urlInput, setUrlInput] = useState("http://127.0.0.1:3000");
  const [tokenInput, setTokenInput] = useState("");

  const connect = useCallback((baseUrl: string, token: string) => {
    const config = { baseUrl, token };
    storeConfig(config);
    setViewState({ type: "connecting", config });

    const client = new IronclawClient(baseUrl, token);
    const timer = setTimeout(() => {
      setViewState({
        type: "error",
        message:
          "Timed out after 10s. Is ironclaw running on " +
          baseUrl +
          "? Make sure IRONCLAW_REBORN_CORS_ORIGINS includes this site's origin.",
      });
    }, 10_000);

    client
      .getSession()
      .then((session) => {
        clearTimeout(timer);
        setViewState({ type: "connected", client, session });
      })
      .catch((err) => {
        clearTimeout(timer);
        setViewState({ type: "error", message: err.message });
      });
  }, []);

  const handleDisconnect = useCallback(() => {
    setViewState({ type: "setup" });
  }, []);

  return (
    <div className="flex h-full w-full">
      {viewState.type === "setup" ? (
        <SetupScreen
          urlInput={urlInput}
          setUrlInput={setUrlInput}
          tokenInput={tokenInput}
          setTokenInput={setTokenInput}
          onConnect={connect}
        />
      ) : viewState.type === "connecting" ? (
        <ConnectingScreen
          onCancel={() => {
            clearConfig();
            setViewState({ type: "setup" });
          }}
        />
      ) : viewState.type === "error" ? (
        <ErrorScreen
          message={viewState.message}
          onRetry={() => {
            clearConfig();
            setViewState({ type: "setup" });
          }}
        />
      ) : (
        <ChatScreen
          client={viewState.client}
          onDisconnect={handleDisconnect}
        />
      )}
    </div>
  );
}

function SetupScreen({
  urlInput,
  setUrlInput,
  tokenInput,
  setTokenInput,
  onConnect,
}: {
  urlInput: string;
  setUrlInput: (v: string) => void;
  tokenInput: string;
  setTokenInput: (v: string) => void;
  onConnect: (url: string, token: string) => void;
}) {
  return (
    <div className="flex w-full items-center justify-center p-6">
      <Card className="w-full max-w-md space-y-6 p-8">
        <div className="space-y-2 text-center">
          <div className="mx-auto flex h-12 w-12 items-center justify-center rounded-xl bg-foreground text-background">
            <Terminal size={22} />
          </div>
          <h1 className="text-xl font-semibold">Connect to IronClaw</h1>
          <p className="text-sm text-muted-foreground">
            Enter your local ironclaw-reborn instance details
          </p>
        </div>

        <div className="space-y-4">
          <div className="space-y-2">
            <label className="text-xs font-medium text-muted-foreground">
              IronClaw URL
            </label>
            <Input
              value={urlInput}
              onChange={(e) => setUrlInput(e.target.value)}
              placeholder="http://127.0.0.1:3000"
            />
          </div>
          <div className="space-y-2">
            <label className="text-xs font-medium text-muted-foreground">
              Auth Token
            </label>
            <Input
              value={tokenInput}
              onChange={(e) => setTokenInput(e.target.value)}
              placeholder="IRONCLAW_REBORN_WEBUI_TOKEN"
              type="password"
              autoComplete="off"
              data-1p-ignore
            />
          </div>
          <Button
            className="w-full"
            onClick={() => onConnect(urlInput, tokenInput)}
            disabled={!urlInput || !tokenInput}
          >
            <Terminal size={14} />
            Connect
          </Button>
        </div>

        <div className="rounded-lg border border-border bg-muted/50 p-3 text-xs text-muted-foreground space-y-1">
          <p className="font-medium text-foreground">Quick setup:</p>
          <p>
            1. Run <code className="rounded bg-secondary px-1 py-0.5">scripts/run-reborn-webui.sh</code>
          </p>
          <p>
            2. Set CORS:{" "}
            <code className="rounded bg-secondary px-1 py-0.5">
              export IRONCLAW_REBORN_CORS_ORIGINS="http://localhost:3001"
            </code>{" "}
            (use your everything.dev host port)
          </p>
          <p>3. Copy the printed login token and paste it here</p>
        </div>
      </Card>
    </div>
  );
}

function ConnectingScreen({
  onCancel,
}: {
  onCancel?: () => void;
}) {
  return (
    <div className="flex w-full flex-col items-center justify-center gap-6">
      <div className="flex items-center gap-3 text-muted-foreground">
        <Loader2 className="h-5 w-5 animate-spin" />
        <span className="text-sm">Connecting to ironclaw...</span>
      </div>
      {onCancel && (
        <Button variant="ghost" size="sm" onClick={onCancel}>
          Cancel
        </Button>
      )}
    </div>
  );
}

function ErrorScreen({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  return (
    <div className="flex w-full items-center justify-center">
      <Card className="max-w-md space-y-4 p-8 text-center">
        <p className="text-sm font-semibold text-destructive">Connection failed</p>
        <p className="text-xs text-muted-foreground">{message}</p>
        <Button variant="outline" onClick={onRetry}>
          Try again
        </Button>
      </Card>
    </div>
  );
}

function ChatScreen({
  client,
  onDisconnect,
}: {
  client: IronclawClient;
  onDisconnect: () => void;
}) {
  const [threads, setThreads] = useState<Thread[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Map<string, ChatMessage[]>>(new Map());
  const [inputText, setInputText] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [streamingEvent, setStreamingEvent] = useState<ChatEvent | null>(null);
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [sseError, setSseError] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const eventSourceRef = useRef<EventSource | null>(null);

  const disconnect = useCallback(() => {
    if (eventSourceRef.current) {
      eventSourceRef.current.close();
    }
    clearConfig();
    onDisconnect();
  }, [onDisconnect]);

  const loadThreads = useCallback(async () => {
    try {
      const result: ThreadList = await client.listThreads(50);
      setThreads(result.data);
    } catch (err) {
      console.error("Failed to load threads:", err);
    }
  }, [client]);

  useEffect(() => {
    loadThreads();
  }, [loadThreads]);

  const openThread = useCallback(
    async (threadId: string) => {
      setActiveThreadId(threadId);
      setStreamingEvent(null);
      setCurrentRunId(null);
      setSseError(false);

      if (eventSourceRef.current) {
        eventSourceRef.current.close();
        eventSourceRef.current = null;
      }

      try {
        const timeline: Timeline = await client.getTimeline(threadId, 100);
        const msgs: ChatMessage[] = timeline.data.map((entry) => ({
          id: entry.id,
          role: entry.role === "user" ? "user" : "assistant",
          content: entry.content ?? "",
          timestamp: entry.createdAt ?? new Date().toISOString(),
        }));
        setMessages((prev) => new Map(prev).set(threadId, msgs));
      } catch (err) {
        console.error("Failed to load timeline:", err);
      }

      const es = client.streamEvents(threadId);
      eventSourceRef.current = es;

      const handleSseEvent = (event: MessageEvent) => {
        try {
          const data: ChatEvent = JSON.parse(event.data);
          setSseError(false);
          setStreamingEvent(data);

          if (data.type === "accepted" && data.ack?.run_id) {
            setCurrentRunId(data.ack.run_id);
          }

          if (data.type === "final_reply" && (data.reply?.text ?? data.reply?.turn_run_id)) {
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
        } catch {
          console.error("Failed to parse SSE event");
        }
      };

      const eventTypes = [
        "accepted", "running", "capability_progress", "capability_activity",
        "capability_display_preview", "gate", "auth_required",
        "final_reply", "cancelled", "failed",
        "projection_snapshot", "projection_update", "keep_alive",
      ];

      for (const type of eventTypes) {
        es.addEventListener(type, handleSseEvent);
      }

      es.onerror = () => {
        if (es.readyState === EventSource.CLOSED || es.readyState === EventSource.CONNECTING) {
          setSseError(true);
        }
      };
    },
    [client],
  );

  useEffect(() => {
    return () => {
      if (eventSourceRef.current) {
        eventSourceRef.current.close();
      }
    };
  }, []);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, activeThreadId, streamingEvent]);

  const createThread = useCallback(async () => {
    try {
      const thread = await client.createThread();
      setThreads((prev) => [thread, ...prev]);
      setActiveThreadId(thread.id);
      setMessages((prev) => new Map(prev).set(thread.id, []));
    } catch (err) {
      toast.error("Failed to create thread");
    }
  }, [client]);

  const deleteThread = useCallback(
    async (threadId: string) => {
      try {
        await client.deleteThread(threadId);
        setThreads((prev) => prev.filter((t) => t.id !== threadId));
        if (activeThreadId === threadId) {
          setActiveThreadId(null);
        }
      } catch {
        toast.error("Failed to delete thread");
      }
    },
    [client, activeThreadId],
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
      await client.sendMessage(activeThreadId, content);
    } catch {
      toast.error("Failed to send message");
    } finally {
      setIsSending(false);
    }
  }, [activeThreadId, inputText, isSending, client]);

  const currentMessages = activeThreadId ? (messages.get(activeThreadId) ?? []) : [];

  return (
    <>
      <div className="flex h-full w-72 shrink-0 flex-col border-r border-border bg-card">
        <div className="flex items-center justify-between border-b border-border px-3 py-2.5">
          <div className="flex items-center gap-1.5">
            <span className="h-1.5 w-1.5 rounded-full bg-green-500" />
            <span className="text-xs font-medium text-muted-foreground">Threads</span>
          </div>
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={disconnect}
              className="text-[10px] text-muted-foreground hover:text-foreground transition-colors px-1 py-0.5 rounded"
              title="Disconnect from ironclaw"
            >
              Disconnect
            </button>
            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={createThread}>
              <Plus size={14} />
            </Button>
          </div>
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
                key={thread.id}
                role="button"
                tabIndex={0}
                onClick={() => openThread(thread.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    openThread(thread.id);
                  }
                }}
                className={`group flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm transition-colors cursor-pointer ${
                  activeThreadId === thread.id
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-muted"
                }`}
              >
                <MessageSquare size={14} className="shrink-0" />
                <span className="flex-1 truncate">
                  {thread.title ?? `Thread ${thread.id.slice(0, 8)}`}
                </span>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    deleteThread(thread.id);
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
                    {streamingEvent.progress?.message ?? "Thinking..."}
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
    </>
  );
}
