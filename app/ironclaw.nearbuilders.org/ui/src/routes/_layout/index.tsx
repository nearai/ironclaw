import { createFileRoute, Link } from "@tanstack/react-router";
import { Loader2, MessageSquare, Plus, Terminal, Trash2, Unplug, Zap, Brain } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { type ApiClient, useApiClient } from "@/app";
import { ChatIdentityBar } from "@/components/chat-identity-bar";
import { ChatInput } from "@/components/chat-input";
import { ChatMessage } from "@/components/chat-message";
import { ChatMessageList } from "@/components/chat-message-list";
import { ChatThreadMeta } from "@/components/chat-thread-meta";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { ThreadState } from "@/hooks/use-thread-state";
import { useIronclawChat } from "@/hooks/use-ironclaw-chat";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";
import { useThreadState } from "@/hooks/use-thread-state";
import type { UIMessage } from "@tanstack/ai/client";

type Thread = Awaited<ReturnType<ApiClient["ironclaw"]["threads"]["list"]>>["data"][number];

export const Route = createFileRoute("/_layout/")({
  component: ChatPage,
});

function ChatArea({
  threadId,
  apiClient,
  initialMessages,
  threadState,
  threadStateError,
  onRebuild,
}: {
  threadId: string;
  apiClient: ApiClient;
  initialMessages: Array<UIMessage>;
  threadState: ThreadState | null;
  threadStateError: string | null;
  onRebuild: () => Promise<void>;
}) {
  const { messages, sendMessage, isLoading, status, error, stop, resolveGate } = useIronclawChat(
    threadId,
    apiClient,
    initialMessages,
  );

  const [isRebuilding, setIsRebuilding] = useState(false);
  const [threadMetaOpen, setThreadMetaOpen] = useState(false);

  const handleRebuild = useCallback(async () => {
    setIsRebuilding(true);
    try {
      await onRebuild();
    } finally {
      setIsRebuilding(false);
    }
  }, [onRebuild]);

  const handleSend = useCallback(
    (content: string) => {
      sendMessage(content);
    },
    [sendMessage],
  );

  const handleApproveTool = useCallback(
    (toolCallId: string, approved: boolean) => {
      resolveGate(toolCallId, approved).catch(() => toast.error("Failed to approve tool"));
    },
    [resolveGate],
  );

  return (
    <>
      <ChatIdentityBar
        threadState={threadState}
        onRebuild={handleRebuild}
        onToggleMeta={() => setThreadMetaOpen((prev) => !prev)}
        isRebuilding={isRebuilding}
      />

      <ChatMessageList
        loading={messages.length === 0 && !threadState}
        empty={messages.length === 0 && !!threadState}
        emptyMessage={
          threadStateError ? "Failed to load thread" : "No messages yet. Send a message to start."
        }
      >
        {messages.map((message) => (
          <ChatMessage
            key={message.id}
            message={message}
            onApproveTool={handleApproveTool}
          />
        ))}
        {isLoading && messages.length > 0 ? (
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Brain size={14} className="animate-pulse" />
            Thinking...
          </div>
        ) : null}
      </ChatMessageList>

      {error ? (
        <div className="mx-auto max-w-3xl px-4 pb-2">
          <p className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
            {error.message}
          </p>
        </div>
      ) : null}

      <ChatInput
        onSend={handleSend}
        onStop={stop}
        placeholder="Type a message..."
        isSending={isLoading || status === "streaming" || status === "submitted"}
      />

      <ChatThreadMeta
        open={threadMetaOpen}
        onOpenChange={setThreadMetaOpen}
        threadState={threadState}
      />
    </>
  );
}

function ChatPage() {
  const apiClient = useApiClient();
  const { status: connectionStatus } = useIronclawStatus();

  const [threads, setThreads] = useState<Thread[]>([]);
  const [threadsLoaded, setThreadsLoaded] = useState(false);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);

  const activeThreadMeta = useMemo(() => {
    if (!activeThreadId) return undefined;
    return threads.find((t) => t.threadId === activeThreadId);
  }, [activeThreadId, threads]);

  const { state: threadState, loading: threadStateLoading, error: threadStateError, rebuild } = useThreadState(activeThreadId, activeThreadMeta);

  const initialMessages = useMemo((): Array<UIMessage> | null => {
    if (!threadState?.messages) return null;
    return threadState.messages
      .filter((m) => m.kind === "user" || m.kind === "assistant")
      .map((m) => ({
        id: m.messageId,
        role: (m.kind === "user" ? "user" : "assistant") as "user" | "assistant",
        parts: [{ type: "text" as const, content: m.content ?? "" }],
        createdAt: m.createdAt ? new Date(m.createdAt) : undefined,
      })) as Array<UIMessage>;
  }, [threadState]);

  const isDisconnected =
    connectionStatus === "disconnected" || connectionStatus === "never-connected";

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

  const openThread = useCallback(async (threadId: string) => {
    setActiveThreadId(threadId);
  }, []);

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
        {activeThreadId && threadStateLoading ? (
          <div className="flex h-full items-center justify-center">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : activeThreadId && initialMessages ? (
          <ChatArea
            key={activeThreadId}
            threadId={activeThreadId}
            apiClient={apiClient}
            initialMessages={initialMessages}
            threadState={threadState}
            threadStateError={threadStateError}
            onRebuild={rebuild}
          />
        ) : activeThreadId && !initialMessages ? (
          <div className="flex h-full items-center justify-center">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
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
