import { createFileRoute, Link } from "@tanstack/react-router";
import { Loader2, MessageSquare, Plus, Send, Terminal, Trash2, Unplug, Zap } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { type ApiClient, useApiClient } from "@/app";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { IronclawSseStatus, StreamEvent } from "@/hooks/use-ironclaw-events";
import { useIronclawEvents } from "@/hooks/use-ironclaw-events";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";

type Thread = Awaited<ReturnType<ApiClient["ironclaw"]["threads"]["list"]>>["data"][number];
type TimelineEntry = Awaited<
  ReturnType<ApiClient["ironclaw"]["threads"]["getTimeline"]>
>["data"][number];

export const Route = createFileRoute("/_layout/")({
  component: ChatPage,
});

function ChatPage() {
  const apiClient = useApiClient();
  const { status: connectionStatus } = useIronclawStatus();

  const [threads, setThreads] = useState<Thread[]>([]);
  const [threadsLoaded, setThreadsLoaded] = useState(false);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [transcripts, setTranscripts] = useState<Record<string, TimelineEntry[]>>({});
  const [optimisticMessages, setOptimisticMessages] = useState<Record<string, TimelineEntry[]>>({});
  const [runStates, setRunStates] = useState<
    Record<string, { runId?: string; activeRunId?: string }>
  >({});
  const [latestEvents, setLatestEvents] = useState<Record<string, StreamEvent>>({});
  const [sseStatus, setSseStatus] = useState<Record<string, IronclawSseStatus>>({});
  const [streamErrors, setStreamErrors] = useState<Record<string, string>>({});
  const [inputText, setInputText] = useState("");
  const [isSending, setIsSending] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const isDisconnected =
    connectionStatus === "disconnected" || connectionStatus === "never-connected";

  const appendTranscript = useCallback((threadId: string, entries: TimelineEntry[]) => {
    setTranscripts((prev) => {
      const existing = prev[threadId] ?? [];
      const known = new Set(existing.map((e) => e.messageId));
      const fresh = entries.filter((e) => !known.has(e.messageId));
      if (fresh.length === 0) return prev;
      return { ...prev, [threadId]: [...existing, ...fresh] };
    });
  }, []);

  const loadThreads = useCallback(async () => {
    try {
      const result = await apiClient.ironclaw.threads.list({ limit: 50 });
      setThreads(result.data);
      setThreadsLoaded(true);
    } catch {
      setThreadsLoaded(true);
    }
  }, [apiClient]);

  useEffect(() => {
    if (!isDisconnected) loadThreads();
  }, [loadThreads, isDisconnected]);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  });

  const handleEvent = useCallback(
    (threadId: string, event: StreamEvent) => {
      setStreamErrors((prev) => {
        if (!prev[threadId]) return prev;
        const n = { ...prev };
        delete n[threadId];
        return n;
      });

      setLatestEvents((prev) => ({ ...prev, [threadId]: event }));

      if (event.type === "keep_alive") return;

      if (event.type === "accepted" && event.ack) {
        setRunStates((prev) => {
          const current = prev[threadId] ?? {};
          const next: { runId?: string; activeRunId?: string } = {};
          if (event.ack!.runId) next.runId = event.ack!.runId;
          if (event.ack!.activeRunId) next.activeRunId = event.ack!.activeRunId;
          return Object.keys(next).length === 0
            ? prev
            : { ...prev, [threadId]: { ...current, ...next } };
        });
        return;
      }

      if (event.type === "final_reply" && event.reply?.text) {
        appendTranscript(threadId, [
          {
            messageId: `reply-${threadId}-${event.reply.turnRunId}`,
            threadId,
            sequence: 0,
            kind: "Assistant",
            status: "completed",
            content: event.reply.text,
            createdAt: event.reply.generatedAt ?? new Date().toISOString(),
            role: "assistant",
          },
        ]);
        setRunStates((prev) => {
          const n = { ...prev };
          delete n[threadId];
          return n;
        });
        return;
      }

      if (event.type === "cancelled" || event.type === "failed") {
        setRunStates((prev) => {
          const n = { ...prev };
          delete n[threadId];
          return n;
        });
      }
    },
    [appendTranscript],
  );

  const { status: currentSseStatus } = useIronclawEvents({
    threadId: activeThreadId,
    enabled: !!activeThreadId && !isDisconnected,
    onEvent: useCallback(
      (envelope) => {
        if (!activeThreadId) return;
        handleEvent(activeThreadId, envelope.event);
      },
      [activeThreadId, handleEvent],
    ),
  });

  useEffect(() => {
    setSseStatus((prev) => {
      if (!activeThreadId) return prev;
      if (currentSseStatus === "connected") {
        const n = { ...prev };
        delete n[activeThreadId];
        return n;
      }
      return { ...prev, [activeThreadId]: currentSseStatus };
    });
  }, [activeThreadId, currentSseStatus]);

  const openThread = useCallback(
    async (threadId: string) => {
      setActiveThreadId(threadId);
      setLatestEvents((prev) => {
        const n = { ...prev };
        delete n[threadId];
        return n;
      });
      setStreamErrors((prev) => {
        const n = { ...prev };
        delete n[threadId];
        return n;
      });

      try {
        const timeline = await apiClient.ironclaw.threads.getTimeline({ id: threadId, limit: 100 });
        setTranscripts((prev) => ({ ...prev, [threadId]: timeline.data }));
        setOptimisticMessages((prev) => {
          const n = { ...prev };
          delete n[threadId];
          return n;
        });
      } catch {
        // timeline load failed; proceed with empty transcript
      }
    },
    [apiClient],
  );

  const sendMessage = useCallback(async () => {
    if (!activeThreadId || !inputText.trim() || isSending) return;

    const content = inputText.trim();
    setInputText("");
    setIsSending(true);

    const optimisticId = `user-${crypto.randomUUID()}`;
    setOptimisticMessages((prev) => ({
      ...prev,
      [activeThreadId]: [
        ...(prev[activeThreadId] ?? []),
        {
          messageId: optimisticId,
          threadId: activeThreadId,
          sequence: 0,
          kind: "User",
          status: "accepted",
          content,
          createdAt: new Date().toISOString(),
          role: "user",
        },
      ],
    }));

    try {
      const accepted = await apiClient.ironclaw.threads.sendMessage({
        id: activeThreadId,
        content,
      });
      if (accepted.runId || accepted.activeRunId) {
        setRunStates((prev) => ({
          ...prev,
          [activeThreadId]: {
            runId: accepted.runId ?? prev[activeThreadId]?.runId,
            activeRunId: accepted.activeRunId,
          },
        }));
      }
    } catch {
      toast.error("Failed to send message");
    } finally {
      setIsSending(false);
    }
  }, [activeThreadId, inputText, isSending, apiClient]);

  const createThread = useCallback(async () => {
    try {
      const result = await apiClient.ironclaw.threads.create({
        clientActionId: `ui-${crypto.randomUUID()}`,
      });
      const entry: Thread = {
        threadId: result.threadId,
        title: result.title,
        scope: { tenantId: "", agentId: "" },
        createdByActorId: "",
      };
      setThreads((prev) => [entry, ...prev]);
      openThread(entry.threadId);
    } catch {
      toast.error("Failed to create thread");
    }
  }, [apiClient, openThread]);

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

  const currentEntries = useMemo(() => {
    if (!activeThreadId) return [];
    return [...(transcripts[activeThreadId] ?? []), ...(optimisticMessages[activeThreadId] ?? [])];
  }, [activeThreadId, transcripts, optimisticMessages]);

  const runState = activeThreadId ? runStates[activeThreadId] : undefined;
  const latestEvent = activeThreadId ? latestEvents[activeThreadId] : undefined;
  const sseError = activeThreadId ? streamErrors[activeThreadId] : undefined;
  const activeSseStatus = activeThreadId ? sseStatus[activeThreadId] : undefined;

  const statusDotClass =
    connectionStatus === "connected"
      ? "bg-[color:var(--near-green)]"
      : connectionStatus === "checking"
        ? "bg-muted-foreground animate-pulse"
        : "bg-destructive";

  return (
    <div className="flex h-full w-full">
      <div className="flex h-full w-72 shrink-0 flex-col border-r border-border bg-card">
        <div className="flex items-center justify-between border-b border-border px-3 py-2.5">
          <div className="flex items-center gap-1.5">
            <span className={`h-1.5 w-1.5 rounded-full shrink-0 ${statusDotClass}`} />
            <span className="text-xs font-medium text-muted-foreground">Threads</span>
          </div>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={createThread}
            disabled={isDisconnected}
            title={isDisconnected ? "Connect IronClaw first" : "New thread"}
          >
            <Plus size={14} />
          </Button>
        </div>

        <ScrollArea className="flex-1">
          <div className="space-y-0.5 p-2">
            {isDisconnected ? (
              <div className="flex flex-col items-center gap-3 px-2 py-6 text-center">
                <div className="flex h-10 w-10 items-center justify-center rounded-full border border-border bg-muted">
                  <Unplug size={16} className="text-muted-foreground" />
                </div>
                <div className="space-y-1">
                  <p className="text-xs font-medium text-foreground">
                    {connectionStatus === "never-connected"
                      ? "IronClaw not set up"
                      : "IronClaw disconnected"}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {connectionStatus === "never-connected"
                      ? "Connect the binary to start chatting"
                      : "Binary unreachable — is it running?"}
                  </p>
                </div>
                <Link
                  to="/ironclaw"
                  className="inline-flex items-center gap-1.5 rounded-full border border-primary/40 bg-primary/5 px-3 py-1 text-xs font-medium text-primary hover:bg-primary/10 transition-colors"
                >
                  <Zap size={10} />
                  {connectionStatus === "never-connected" ? "Set up IronClaw" : "Setup guide"}
                </Link>
              </div>
            ) : threadsLoaded && threads.length === 0 ? (
              <p className="px-2 py-4 text-center text-xs text-muted-foreground">
                No threads yet. Create one to start chatting.
              </p>
            ) : (
              threads.map((thread) => (
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
              ))
            )}
          </div>
        </ScrollArea>
      </div>

      <div className="flex flex-1 flex-col min-w-0">
        {activeThreadId ? (
          <>
            <ScrollArea className="flex-1 p-4">
              <div className="mx-auto max-w-3xl space-y-4">
                {currentEntries.map((entry) => (
                  <div
                    key={entry.messageId}
                    className={`flex ${entry.role === "user" ? "justify-end" : "justify-start"}`}
                  >
                    <div
                      className={`max-w-[80%] rounded-xl px-4 py-2.5 text-sm ${
                        entry.role === "user"
                          ? "bg-primary text-primary-foreground"
                          : "bg-muted text-foreground"
                      }`}
                    >
                      <p className="whitespace-pre-wrap">{entry.content ?? ""}</p>
                    </div>
                  </div>
                ))}
                {(runState?.runId || runState?.activeRunId) &&
                  latestEvent?.type !== "running" &&
                  latestEvent?.type !== "gate" && (
                    <div className="flex items-center gap-2 text-xs text-muted-foreground">
                      <Loader2 className="h-3 w-3 animate-spin" />
                      {latestEvent?.type === "accepted"
                        ? "Accepted, waiting for response..."
                        : "Message received, waiting..."}
                    </div>
                  )}
                {latestEvent?.type === "running" && latestEvent?.progress && (
                  <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    {latestEvent.progress.kind || "Thinking..."}
                  </div>
                )}
                {latestEvent?.type === "gate" && latestEvent?.prompt && (
                  <div className="rounded-lg border border-[color:var(--chart-3)]/30 bg-[color:var(--chart-3)]/5 px-4 py-3 text-xs text-[color:var(--chart-3)]">
                    <p className="font-medium mb-0.5">{latestEvent.prompt.headline}</p>
                    <p>{latestEvent.prompt.body}</p>
                  </div>
                )}
                {latestEvent?.type === "auth_required" && latestEvent?.authPrompt && (
                  <div className="rounded-lg border border-[color:var(--chart-3)]/30 bg-[color:var(--chart-3)]/5 px-4 py-3 text-xs text-[color:var(--chart-3)]">
                    <p className="font-medium mb-0.5">{latestEvent.authPrompt.headline}</p>
                    <p>{latestEvent.authPrompt.body}</p>
                    {latestEvent.authPrompt.authorizationUrl && (
                      <a
                        href={latestEvent.authPrompt.authorizationUrl}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="mt-1 inline-flex items-center gap-1 text-primary underline"
                      >
                        Authorize
                      </a>
                    )}
                  </div>
                )}
                {latestEvent?.type === "cancelled" && (
                  <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-xs text-destructive">
                    Run cancelled
                  </div>
                )}
                {latestEvent?.type === "failed" && (
                  <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-xs text-destructive">
                    Run failed
                    {(() => {
                      const f = latestEvent.runState?.failure;
                      if (f && typeof f === "object") {
                        const msg = (f as Record<string, unknown>).message;
                        return msg ? (
                          <span className="block mt-0.5 opacity-80">{String(msg)}</span>
                        ) : null;
                      }
                      return null;
                    })()}
                  </div>
                )}
                {(latestEvent?.type === "capability_activity" ||
                  latestEvent?.type === "capability_display_preview") && (
                  <div className="flex items-center gap-2 rounded-lg border border-border/50 bg-muted/30 px-4 py-2.5 text-xs text-muted-foreground">
                    <Loader2 className="h-3 w-3 animate-spin shrink-0" />
                    <span>
                      {latestEvent.type === "capability_display_preview"
                        ? (latestEvent.preview?.title ?? "Capability running")
                        : "Capability activity running"}
                    </span>
                  </div>
                )}
                {latestEvent?.type === "projection_snapshot" ||
                latestEvent?.type === "projection_update"
                  ? null
                  : null}
                {activeSseStatus === "reconnecting" && !sseError && (
                  <div className="flex items-center gap-2 rounded-lg border border-[color:var(--chart-3)]/30 bg-[color:var(--chart-3)]/5 px-4 py-3 text-xs text-[color:var(--chart-3)]">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    Reconnecting to event stream...
                  </div>
                )}
                {sseError && (
                  <div className="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-xs text-destructive">
                    <Unplug className="h-3 w-3" />
                    {sseError}
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
                  placeholder={isDisconnected ? "IronClaw not connected" : "Type a message..."}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !e.shiftKey) {
                      e.preventDefault();
                      sendMessage();
                    }
                  }}
                  disabled={isSending || isDisconnected}
                />
                <Button
                  size="icon"
                  onClick={sendMessage}
                  disabled={!inputText.trim() || isSending || isDisconnected}
                >
                  <Send size={14} />
                </Button>
              </div>
            </div>
          </>
        ) : (
          <div className="flex h-full items-center justify-center">
            {isDisconnected ? (
              <div className="text-center space-y-4 max-w-xs px-4">
                <div className="flex h-14 w-14 items-center justify-center rounded-full border border-border bg-muted mx-auto">
                  <Unplug className="h-6 w-6 text-muted-foreground" />
                </div>
                <div className="space-y-1.5">
                  <p className="text-sm font-semibold text-foreground">
                    {connectionStatus === "never-connected"
                      ? "IronClaw not connected"
                      : "Connection lost"}
                  </p>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    {connectionStatus === "never-connected"
                      ? "Run the IronClaw binary locally, then return here to start chatting."
                      : "The IronClaw binary stopped responding. Check that it's still running."}
                  </p>
                </div>
                <Link
                  to="/ironclaw"
                  className="inline-flex items-center gap-2 rounded-full border border-primary/40 bg-primary/5 px-4 py-2 text-sm font-medium text-primary hover:bg-primary/10 transition-colors"
                >
                  <Zap size={14} />
                  {connectionStatus === "never-connected" ? "Set up IronClaw" : "Setup guide"}
                </Link>
              </div>
            ) : (
              <div className="text-center">
                <Terminal className="mx-auto h-8 w-8 text-muted-foreground" />
                <p className="mt-2 text-sm text-muted-foreground">
                  Select a thread or create a new one
                </p>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
