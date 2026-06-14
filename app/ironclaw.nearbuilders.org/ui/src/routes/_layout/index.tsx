import { createFileRoute, Link } from "@tanstack/react-router";
import { MessageSquare, Plus, Terminal, Trash2, Unplug, Zap } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useApiClient } from "@/app";
import { ChatIdentityBar } from "@/components/chat-identity-bar";
import { ChatInput } from "@/components/chat-input";
import { ChatMessage } from "@/components/chat-message";
import { ChatMessageList } from "@/components/chat-message-list";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";
import { useConversationThreads, useConversationMessages, useSendConversationMessage, useConversationStream, type ConversationThread } from "@/hooks/use-conversation";

export const Route = createFileRoute("/_layout/")({
  component: ChatPage,
});

interface ThreadMeta {
  threadId: string;
  title: string | null;
  scope: { tenantId: string; agentId: string; projectId?: string };
  createdByActorId: string;
}

function ChatArea({
  threadId,
  threadMeta,
  threadMetaError,
}: {
  threadId: string;
  threadMeta: ThreadMeta | null;
  threadMetaError: string | null;
}) {
  const sendMessage = useSendConversationMessage(threadId);
  const [pendingRun, setPendingRun] = useState<{ runId: string; content: string } | null>(null);
  const knownMessageIdsRef = useRef<Set<string>>(new Set());

  const { data, isLoading } = useConversationMessages(threadId);

  const allMessages = useMemo(() => {
    return data?.pages.flatMap((p) => p.messages) ?? [];
  }, [data]);

  const handleSend = useCallback(
    (content: string) => {
      if (!content.trim()) return;
      const clientActionId = `ui-${crypto.randomUUID()}`;
      setPendingRun({ runId: clientActionId, content });
      sendMessage.mutate({ content, clientActionId });
    },
    [sendMessage],
  );

  const handleRebuild = useCallback(async () => {
    // no-op: stream subscription auto-refetches
  }, []);

  const handleToggleMeta = useCallback(() => {
    // no-op: thread meta sheet not wired yet
  }, []);

  const userMessage = useMemo(() => {
    if (!pendingRun) return null;
    return {
      id: `pending-${pendingRun.runId}`,
      threadId,
      role: "user" as const,
      text: pendingRun.content,
      createdAt: new Date().toISOString(),
      status: "submitted" as const,
      sequence: allMessages.length > 0 ? allMessages[allMessages.length - 1]!.sequence + 1 : 1,
      runId: pendingRun.runId,
    };
  }, [pendingRun, threadId, allMessages]);

  const displayMessages = useMemo(() => {
    const msgs = [...allMessages];
    if (userMessage && pendingRun) {
      const alreadyShown = allMessages.some(
        (m) => m.role === "user" && m.text === pendingRun.content,
      );
      if (!alreadyShown) msgs.push(userMessage);
    }
    return msgs;
  }, [allMessages, userMessage, pendingRun]);

  const hasNewAssistantMessage = useMemo(() => {
    if (!pendingRun) return false;
    return allMessages.some((m) => m.role === "assistant" && !knownMessageIdsRef.current.has(m.id));
  }, [allMessages, pendingRun]);

  const isThinking = sendMessage.isPending || (pendingRun !== null && !hasNewAssistantMessage);

  useEffect(() => {
    if (!sendMessage.isPending && pendingRun && hasNewAssistantMessage) {
      setPendingRun(null);
    }
  }, [sendMessage.isPending, pendingRun, hasNewAssistantMessage]);

  useEffect(() => {
    for (const msg of allMessages) {
      knownMessageIdsRef.current.add(msg.id);
    }
  }, [allMessages]);

  return (
    <>
      <ChatIdentityBar
        threadState={
          threadMeta
            ? {
                thread: {
                  threadId: threadMeta.threadId,
                  title: threadMeta.title,
                  scope: {
                    tenantId: threadMeta.scope.tenantId,
                    agentId: threadMeta.scope.agentId,
                    projectId: threadMeta.scope.projectId,
                  },
                  createdByActorId: threadMeta.createdByActorId,
                },
                messages: [],
              }
            : null
        }
        onRebuild={handleRebuild}
        onToggleMeta={handleToggleMeta}
      />

      <ChatMessageList
        loading={isLoading}
        empty={displayMessages.length === 0 && !isLoading}
        emptyMessage={threadMetaError ? "Failed to load thread" : "No messages yet. Send a message to start."}
      >
        {displayMessages.map((message) => (
          <ChatMessage
            key={message.id}
            status={message.status}
            message={{
              id: message.id,
              role: message.role,
              parts: [{ type: "text" as const, content: message.text }],
              createdAt: message.createdAt ? new Date(message.createdAt) : undefined,
            }}
          />
        ))}
        {isThinking ? (
          <div className="flex items-start gap-3 px-4 py-2">
            <div className="rounded-2xl rounded-bl-sm bg-muted px-4 py-2.5">
              <div className="flex items-center gap-1">
                <span className="h-2 w-2 animate-bounce rounded-full bg-muted-foreground/40 [animation-delay:0ms]" />
                <span className="h-2 w-2 animate-bounce rounded-full bg-muted-foreground/40 [animation-delay:150ms]" />
                <span className="h-2 w-2 animate-bounce rounded-full bg-muted-foreground/40 [animation-delay:300ms]" />
              </div>
            </div>
          </div>
        ) : null}
      </ChatMessageList>

      <ChatInput
        onSend={handleSend}
        placeholder="Type a message..."
        isSending={isThinking}
      />
    </>
  );
}

function ChatPage() {
  const apiClient = useApiClient();
  const { status: connectionStatus } = useIronclawStatus();

  const isDisconnected =
    connectionStatus === "disconnected" || connectionStatus === "never-connected";

  const threadsQuery = useConversationThreads();
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);

  const threads = useMemo(() => {
    return (threadsQuery.data?.pages.flatMap((p) => p.threads) ?? []) as ConversationThread[];
  }, [threadsQuery.data]);

  const activeThreadMeta = useMemo(() => {
    if (!activeThreadId) return null;
    const found = threads.find((t) => t.threadId === activeThreadId);
    if (!found) return null;
    return {
      threadId: found.threadId,
      title: found.title,
      scope: { tenantId: found.tenantId, agentId: found.agentId, projectId: found.projectId ?? undefined },
      createdByActorId: found.createdByActorId,
    } satisfies ThreadMeta;
  }, [activeThreadId, threads]);

  useConversationStream(activeThreadId);

  const openThread = useCallback(async (threadId: string) => {
    setActiveThreadId(threadId);
  }, []);

  const createThread = useCallback(async () => {
    try {
      const result = await apiClient.ironclaw.threads.create({
        clientActionId: `ui-${crypto.randomUUID()}`,
      });
      threadsQuery.refetch();
      openThread(result.threadId);
    } catch {
      toast.error("Failed to create thread");
    }
  }, [apiClient, openThread, threadsQuery]);

  const deleteThread = useCallback(
    async (threadId: string) => {
      try {
        await apiClient.ironclaw.threads.delete({ id: threadId });
        threadsQuery.refetch();
        if (activeThreadId === threadId) {
          setActiveThreadId(null);
        }
      } catch {
        toast.error("Failed to delete thread");
      }
    },
    [apiClient, activeThreadId, threadsQuery],
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
            ) : threadsQuery.isSuccess && threads.length === 0 ? (
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
          <ChatArea
            key={activeThreadId}
            threadId={activeThreadId}
            threadMeta={activeThreadMeta}
            threadMetaError={null}
          />
        ) : isDisconnected ? (
          <div className="flex h-full items-center justify-center">
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
          </div>
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
