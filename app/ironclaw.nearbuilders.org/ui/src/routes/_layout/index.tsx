import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import type { UIMessage } from "@tanstack/ai-react";
import type { MessagePart } from "@tanstack/ai";
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
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";
import { useIronclawChat } from "@/hooks/use-ironclaw-chat";
import { useConversationThreads, type ConversationThread } from "@/hooks/use-conversation";
import { openIronclawEventSource } from "@/lib/ironclaw-sse";

export const Route = createFileRoute("/_layout/")({
  component: ChatPage,
});

interface ThreadMeta {
  threadId: string;
  title: string | null;
  scope: { tenantId: string; agentId: string; projectId?: string };
  createdByActorId: string;
}

function restMessageToParts(role: string, text: string): MessagePart[] {
  if (role !== "assistant" || !text) {
    return [{ type: "text" as const, content: text.trim() }];
  }

  try {
    const parsed = JSON.parse(text);
    if (!parsed || typeof parsed !== "object" || parsed.version !== 1) {
      return [{ type: "text" as const, content: text.trim() }];
    }

    if (parsed.capability_id && parsed.invocation_id) {
      const outputText = parsed.output_preview ?? parsed.output_summary ?? "";
      const isError = parsed.status === "failed" || parsed.status === "error";
      const toolCallId = parsed.invocation_id as string;
      return [
        {
          type: "tool-call" as const,
          id: toolCallId,
          name: (parsed.title as string) ?? (parsed.capability_id as string),
          arguments: "{}",
          state: "complete" as const,
        },
        {
          type: "tool-result" as const,
          toolCallId,
          content: typeof outputText === "string" ? outputText : JSON.stringify(outputText),
          state: isError ? ("error" as const) : ("complete" as const),
        },
      ];
    }

    if (parsed.result_ref) {
      return [];
    }
  } catch {
    // not JSON — fall through to plain text
  }

  return [{ type: "text" as const, content: text.trim() }];
}

function ChatArea({
  threadId,
  threadMeta,
  threadMetaError,
  onOpenMobileSidebar,
  onToggleDesktopSidebar,
}: {
  threadId: string;
  threadMeta: ThreadMeta | null;
  threadMetaError: string | null;
  onOpenMobileSidebar?: () => void;
  onToggleDesktopSidebar?: () => void;
}) {
  const apiClient = useApiClient();
  const [initialMessages, setInitialMessages] = useState<UIMessage[]>([]);
  const [loadingInitial, setLoadingInitial] = useState(true);

  const fetchThreadMessages = useCallback(async () => {
    const page = await (apiClient as any).conversation.getMessages({ threadId, limit: 100 });
    return (page?.messages ?? []).map((m: any) => ({
      id: m.id,
      role: m.role,
      parts: restMessageToParts(m.role, m.text ?? ""),
      createdAt: m.createdAt ? new Date(m.createdAt) : undefined,
    })) as UIMessage[];
  }, [apiClient, threadId]);

  useEffect(() => {
    let cancelled = false;
    setLoadingInitial(true);
    setInitialMessages([]);

    (async () => {
      try {
        const messages = await fetchThreadMessages();
        if (cancelled) return;
        setInitialMessages(messages);
      } catch (err) {
        console.error("[ChatArea] initial message load failed", err);
      }
      if (!cancelled) setLoadingInitial(false);
    })();

    return () => {
      cancelled = true;
    };
  }, [fetchThreadMessages]);

  const chat = useIronclawChat(threadId, apiClient, initialMessages);
  const syncKnownIdsRef = useRef<Set<string>>(new Set());
  const isLoadingRef = useRef(chat.isLoading);

  useEffect(() => {
    isLoadingRef.current = chat.isLoading;
  }, [chat.isLoading]);

  useEffect(() => {
    syncKnownIdsRef.current = new Set(chat.messages.map((m) => m.id));
  }, [chat.messages]);

  useEffect(() => {
    const syncMessages = async () => {
      if (isLoadingRef.current) return;
      try {
        const fresh = await fetchThreadMessages();
        const known = syncKnownIdsRef.current;
        const hasNew = fresh.some((m) => !known.has(m.id));
        if (hasNew) {
          chat.setMessages(fresh);
        }
      } catch (err) {
        console.error("[ChatArea] projection sync failed", err);
      }
    };

    const handle = openIronclawEventSource({
      threadId,
      onSnapshot: syncMessages,
      onUpdate: syncMessages,
      onEvent: () => {},
      onError: () => {},
    });

    return () => handle.close();
  }, [threadId, fetchThreadMessages, chat.setMessages]);

  useEffect(() => {
    if (!chat.error || chat.isLoading) return;

    void (async () => {
      try {
        const fresh = await fetchThreadMessages();
        const known = syncKnownIdsRef.current;
        const hasNew = fresh.some((m) => !known.has(m.id));
        if (hasNew) {
          chat.setMessages(fresh);
        }
      } catch (err) {
        console.error("[ChatArea] error recovery sync failed", err);
      }
    })();
  }, [chat.error, chat.isLoading, chat.setMessages, fetchThreadMessages]);

  const handleSend = useCallback(
    (content: string) => {
      if (!content.trim() || chat.isLoading) return;
      chat.sendMessage(content);
    },
    [chat.sendMessage, chat.isLoading],
  );

  const handleRebuild = useCallback(async () => {}, []);

  const handleToggleMeta = useCallback(() => {}, []);

  const messages = chat.messages;
  const isThinking = chat.isLoading;
  const isEmpty = messages.length === 0 && !isThinking && !loadingInitial;

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
        onOpenMobileSidebar={onOpenMobileSidebar}
        onToggleDesktopSidebar={onToggleDesktopSidebar}
        activeThreadTitle={threadMeta?.title ?? `Thread ${threadId.slice(0, 8)}`}
      />

      <ChatMessageList
        loading={loadingInitial}
        streamLoading={isThinking}
        empty={isEmpty}
        emptyMessage={threadMetaError ? "Failed to load thread" : "No messages yet. Send a message to start."}
      >
        {messages.filter((m) => m.parts.length > 0).map((message) => (
          <ChatMessage key={message.id} message={message} onApproveTool={chat.resolveGate} />
        ))}
        {isThinking ? (
          <div className="flex items-start gap-3">
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
        onStop={chat.stop}
        placeholder="Type a message..."
        isSending={isThinking}
      />
    </>
  );
}

function ChatPage() {
  const apiClient = useApiClient();
  const { status: connectionStatus } = useIronclawStatus();
  const navigate = useNavigate();

  useEffect(() => {
    if (connectionStatus === "never-connected") {
      navigate({ to: "/setup", replace: true });
    }
  }, [connectionStatus, navigate]);

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

  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [sheetOpen, setSheetOpen] = useState(false);

  const statusDotClass =
    connectionStatus === "connected"
      ? "bg-[color:var(--near-green)]"
      : connectionStatus === "checking"
        ? "bg-muted-foreground animate-pulse"
        : "bg-destructive";

  const threadListContent = (
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
              to="/setup"
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
              onClick={() => {
                openThread(thread.threadId);
                setSheetOpen(false);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  openThread(thread.threadId);
                  setSheetOpen(false);
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
  );

  const sidebarHeader = (
    <div className="flex items-center justify-between border-b border-border px-3 py-2.5 shrink-0">
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
  );

  return (
    <div className="flex h-full w-full overflow-hidden">
      <div
        className={`hidden lg:flex h-full shrink-0 flex-col border-r border-border bg-card transition-all duration-200 overflow-hidden ${
          sidebarOpen ? "w-72" : "w-0"
        }`}
      >
        {sidebarHeader}
        {threadListContent}
      </div>

      <Sheet open={sheetOpen} onOpenChange={setSheetOpen}>
        <SheetContent side="left" className="flex w-72 flex-col p-0 lg:hidden">
          <SheetHeader className="sr-only">
            <SheetTitle>Threads</SheetTitle>
          </SheetHeader>
          {sidebarHeader}
          {threadListContent}
        </SheetContent>
      </Sheet>

      <div className="flex flex-1 flex-col min-w-0 h-full overflow-hidden">
        {activeThreadId ? (
          <ChatArea
            key={activeThreadId}
            threadId={activeThreadId}
            threadMeta={activeThreadMeta}
            threadMetaError={null}
            onOpenMobileSidebar={() => setSheetOpen(true)}
            onToggleDesktopSidebar={() => setSidebarOpen(!sidebarOpen)}
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
                to="/setup"
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
