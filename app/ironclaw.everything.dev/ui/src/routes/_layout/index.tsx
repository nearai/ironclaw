import type { UIMessage } from "@tanstack/ai-react";
import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import {
  ChevronLeft,
  ChevronRight,
  MessageSquare,
  Plus,
  RefreshCw,
  ShieldCheck,
  Trash2,
  Unplug,
  Zap,
} from "lucide-react";
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
import { type ConversationThread, useConversationThreads } from "@/hooks/use-conversation";
import type { AttachmentLimits } from "@/lib/attachments";
import { useThreadChat } from "@/hooks/use-thread-chat";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";
import { useVerboseMode } from "@/hooks/use-verbose-mode";
import { messagesToUIMessages } from "@/lib/ironclaw-message-parts";

export const Route = createFileRoute("/_layout/")({
  component: ChatPage,
});

const SIDEBAR_MIN_WIDTH = 200;
const SIDEBAR_MAX_WIDTH = 480;
const SIDEBAR_DEFAULT_WIDTH = 272;
const SIDEBAR_COLLAPSED_WIDTH = 40;

interface ThreadMeta {
  threadId: string;
  title: string | null;
  scope: { tenantId: string; agentId: string; projectId?: string };
  createdByActorId: string;
}

function ChatArea(props: {
  threadId: string;
  threadMeta: ThreadMeta | null;
  threadMetaError: string | null;
  onOpenMobileSidebar?: () => void;
  onToggleDesktopSidebar?: () => void;
  attachmentCapabilities?: AttachmentLimits | null;
  verbose: boolean;
  onToggleVerbose: () => void;
}) {
  const apiClient = useApiClient();
  const [initialMessages, setInitialMessages] = useState<UIMessage[]>([]);
  const [loadingInitial, setLoadingInitial] = useState(true);
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const loginTicket = params.get("loginTicket");
    if (!loginTicket) return;
    params.delete("loginTicket");
    window.history.replaceState(null, "", window.location.pathname + (params.toString() ? `?${params}` : ""));
    apiClient.ironclaw.auth.exchangeLoginTicket({ loginTicket }).catch(() => {});
  }, [apiClient]);

  const fetchThreadMessages = useCallback(async () => {
    const page = await apiClient.conversation.getMessages({ threadId: props.threadId, limit: 100 });
    return messagesToUIMessages(page.messages ?? []);
  }, [apiClient, props.threadId]);

  useEffect(() => {
    let cancelled = false;
    setLoadingInitial(true);
    setInitialMessages([]);
    (async () => {
      try {
        const messages = await fetchThreadMessages();
        if (!cancelled) setInitialMessages(messages);
      } catch {}
      if (!cancelled) setLoadingInitial(false);
    })();
    return () => { cancelled = true; };
  }, [fetchThreadMessages]);

  if (loadingInitial) {
    return (
      <>
        <ChatIdentityBar
          threadState={null}
          onOpenMobileSidebar={props.onOpenMobileSidebar}
          onToggleDesktopSidebar={props.onToggleDesktopSidebar}
          verbose={props.verbose}
          onToggleVerbose={props.onToggleVerbose}
        />
        <ChatMessageList loading><div /></ChatMessageList>
      </>
    );
  }

  return <ChatAreaCore {...props} initialMessages={initialMessages} />;
}

function ChatAreaCore({
  threadId,
  threadMeta,
  threadMetaError,
  onOpenMobileSidebar,
  onToggleDesktopSidebar,
  attachmentCapabilities,
  verbose,
  onToggleVerbose,
  initialMessages,
}: {
  threadId: string;
  threadMeta: ThreadMeta | null;
  threadMetaError: string | null;
  onOpenMobileSidebar?: () => void;
  onToggleDesktopSidebar?: () => void;
  attachmentCapabilities?: AttachmentLimits | null;
  verbose: boolean;
  onToggleVerbose: () => void;
  initialMessages: UIMessage[];
}) {
  const chat = useThreadChat({ threadId, initialMessages });
  const isBusy = chat.isLoading;

  const handleSend = useCallback(
    (content: string) => {
      if (!content.trim() || isBusy) return;
      chat.sendMessage(content);
    },
    [chat.sendMessage, isBusy],
  );

  const firstPendingApproval = chat.pendingApprovals[0];
  const showLoading = chat.isLoading || chat.error != null;

  const threadState = threadMeta
    ? { thread: { threadId: threadMeta.threadId, title: threadMeta.title, scope: { tenantId: threadMeta.scope.tenantId, agentId: threadMeta.scope.agentId, projectId: threadMeta.scope.projectId }, createdByActorId: threadMeta.createdByActorId }, messages: [] }
    : null;

  return (
    <>
      <ChatIdentityBar
        threadState={threadState}
        onOpenMobileSidebar={onOpenMobileSidebar}
        onToggleDesktopSidebar={onToggleDesktopSidebar}
        activeThreadTitle={threadMeta?.title ?? `Thread ${threadId.slice(0, 8)}`}
        verbose={verbose}
        onToggleVerbose={onToggleVerbose}
        onCopyConversation={chat.copyConversation}
      />
      {firstPendingApproval ? (
        <div className="flex w-full items-center gap-2 border-b border-amber-500/20 bg-amber-500/5 px-4 py-2 text-xs text-amber-600">
          <ShieldCheck size={12} className="shrink-0" />
          <span className="flex-1 min-w-0 truncate">{firstPendingApproval.headline}</span>
          <Button
            size="sm"
            variant="outline"
            className="h-7 shrink-0 border-amber-500/30 bg-amber-500/10 px-2.5 text-xs font-medium text-amber-600 hover:bg-amber-500/20"
            onClick={() => chat.runId && chat.resolveGate(chat.runId, firstPendingApproval.gateRef, true)}
          >
            Approve
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="h-7 shrink-0 border-amber-500/30 bg-amber-500/10 px-2.5 text-xs font-medium text-amber-600 hover:bg-amber-500/20"
            onClick={() => chat.runId && chat.resolveGate(chat.runId, firstPendingApproval.gateRef, false)}
          >
            Deny
          </Button>
        </div>
      ) : null}

      <ChatMessageList
        streamLoading={isBusy}
        empty={initialMessages.length === 0 && !isBusy}
        emptyMessage={threadMetaError ? "Failed to load thread" : "No messages yet. Send a message to start."}
      >
        {chat.messages.filter((m: any) => m.parts.length > 0).map((message: any) => (
          <ChatMessage key={message.id} message={message} verbose={verbose} />
        ))}
        {showLoading ? (
          <div className="flex items-end gap-2">
            <div className="shrink-0 w-6 h-6 sm:w-7 sm:h-7 rounded-full overflow-hidden border border-border bg-card flex items-center justify-center mb-0.5">
              <img src="/logo.png" alt="IronClaw" className="w-full h-full object-contain p-0.5" />
            </div>
            <div className="rounded-2xl rounded-bl-sm bg-muted px-4 py-2.5">
              <div className="flex items-center gap-1">
                <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/40 [animation-delay:0ms]" />
                <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/40 [animation-delay:150ms]" />
                <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/40 [animation-delay:300ms]" />
              </div>
            </div>
          </div>
        ) : null}
      </ChatMessageList>

      <ChatInput onSend={handleSend} onStop={chat.stop} placeholder="Type a message..." isSending={isBusy} attachmentCapabilities={attachmentCapabilities} />
    </>
  );
}

function ChatPage() {
  const apiClient = useApiClient();
  const {
    status: connectionStatus,
    refetch: refetchStatus,
    attachmentCapabilities,
  } = useIronclawStatus();
  const navigate = useNavigate();
  const { verbose, toggle: toggleVerbose } = useVerboseMode();

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
    const all = (threadsQuery.data?.pages.flatMap((p) => p.threads) ?? []) as ConversationThread[];
    return all.sort((a, b) => {
      const aTime = a.updatedAt ?? a.createdAt ?? "";
      const bTime = b.updatedAt ?? b.createdAt ?? "";
      return bTime.localeCompare(aTime);
    });
  }, [threadsQuery.data]);

  const activeThreadMeta = useMemo(() => {
    if (!activeThreadId) return null;
    const found = threads.find((t) => t.threadId === activeThreadId);
    if (!found) return null;
    return {
      threadId: found.threadId,
      title: found.title,
      scope: {
        tenantId: found.tenantId,
        agentId: found.agentId,
        projectId: found.projectId ?? undefined,
      },
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
  const [sidebarWidth, setSidebarWidth] = useState(SIDEBAR_DEFAULT_WIDTH);
  const [sheetOpen, setSheetOpen] = useState(false);
  const isResizing = useRef(false);
  const startX = useRef(0);
  const startWidth = useRef(0);

  const handleResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    isResizing.current = true;
    startX.current = e.clientX;
    startWidth.current = sidebarWidth;

    const onMouseMove = (ev: MouseEvent) => {
      if (!isResizing.current) return;
      const delta = ev.clientX - startX.current;
      const newWidth = Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, startWidth.current + delta));
      setSidebarWidth(newWidth);
      if (newWidth <= SIDEBAR_MIN_WIDTH + 20 && !sidebarOpen) setSidebarOpen(true);
    };

    const onMouseUp = () => {
      isResizing.current = false;
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
    };

    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
  }, [sidebarWidth, sidebarOpen]);

  const statusDotClass =
    connectionStatus === "connected"
      ? "bg-[color:var(--near-green)]"
      : connectionStatus === "checking"
        ? "bg-muted-foreground animate-pulse"
        : "bg-destructive";

  const threadListContent = (
    <ScrollArea className="flex-1 min-h-0">
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
              className={`group flex w-full items-center gap-2 rounded-lg px-3 py-2.5 text-left text-sm transition-colors cursor-pointer touch-manipulation ${
                activeThreadId === thread.threadId
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-muted active:bg-muted"
              }`}
            >
              <MessageSquare size={14} className="shrink-0" />
              <span className="flex-1 truncate text-xs">
                {thread.title ?? `Thread ${thread.threadId.slice(0, 8)}`}
              </span>
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  deleteThread(thread.threadId);
                }}
                className="shrink-0 p-1 -m-1 opacity-0 group-hover:opacity-100 focus:opacity-100 transition-opacity touch-manipulation"
                aria-label="Delete thread"
              >
                <Trash2 size={12} className="text-muted-foreground/40 hover:text-destructive transition-colors" />
              </button>
            </div>
          ))
        )}
      </div>
    </ScrollArea>
  );

  const desktopSidebarHeader = (
    <div className="flex items-center justify-between border-b border-border px-3 py-2.5 shrink-0">
      <div className="flex items-center gap-1.5 min-w-0">
        <span className={`h-1.5 w-1.5 rounded-full shrink-0 ${statusDotClass}`} />
        {sidebarOpen && (
          <span className="text-xs font-medium text-muted-foreground truncate">Threads</span>
        )}
      </div>
      <div className="flex items-center gap-1 shrink-0">
        {sidebarOpen && (
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
        )}
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0"
          onClick={() => setSidebarOpen((v) => !v)}
          title={sidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
        >
          {sidebarOpen ? <ChevronLeft size={14} /> : <ChevronRight size={14} />}
        </Button>
      </div>
    </div>
  );

  const mobileSidebarHeader = (
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
        className="hidden lg:flex h-full shrink-0 flex-col border-r border-border bg-card transition-[width] duration-200 overflow-hidden relative"
        style={{ width: sidebarOpen ? sidebarWidth : SIDEBAR_COLLAPSED_WIDTH }}
      >
        {desktopSidebarHeader}
        {sidebarOpen ? (
          <>
            {threadListContent}
            <div
              onMouseDown={handleResizeStart}
              className="absolute right-0 top-0 bottom-0 w-1 cursor-col-resize hover:bg-primary/30 active:bg-primary/50 transition-colors z-10 group"
              title="Drag to resize"
            >
              <div className="absolute right-0 top-0 bottom-0 w-3 -translate-x-1" />
            </div>
          </>
        ) : (
          <div className="flex flex-col items-center gap-1 py-2">
            {threads.slice(0, 8).map((thread) => (
              <button
                key={thread.threadId}
                type="button"
                onClick={() => {
                  openThread(thread.threadId);
                  setSidebarOpen(true);
                }}
                className={`w-8 h-8 rounded-lg flex items-center justify-center transition-colors touch-manipulation ${
                  activeThreadId === thread.threadId
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-muted"
                }`}
                title={thread.title ?? `Thread ${thread.threadId.slice(0, 8)}`}
              >
                <MessageSquare size={13} />
              </button>
            ))}
            {!isDisconnected && (
              <button
                type="button"
                onClick={createThread}
                className="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted transition-colors touch-manipulation mt-1"
                title="New thread"
              >
                <Plus size={13} />
              </button>
            )}
          </div>
        )}
      </div>

      <Sheet open={sheetOpen} onOpenChange={setSheetOpen}>
        <SheetContent side="left" className="flex flex-col p-0 lg:hidden w-[min(320px,85vw)]">
          <SheetHeader className="sr-only">
            <SheetTitle>Threads</SheetTitle>
          </SheetHeader>
          {mobileSidebarHeader}
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
            onToggleDesktopSidebar={() => setSidebarOpen((v) => !v)}
            attachmentCapabilities={attachmentCapabilities}
            verbose={verbose}
            onToggleVerbose={toggleVerbose}
          />
        ) : isDisconnected ? (
          <div className="flex h-full items-center justify-center px-4">
            <div className="text-center space-y-4 max-w-xs w-full">
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
              <div className="flex flex-col items-center gap-2">
                {connectionStatus === "disconnected" && (
                  <button
                    type="button"
                    onClick={() => refetchStatus()}
                    className="inline-flex items-center gap-2 rounded-full border border-primary/40 bg-primary/5 px-4 py-2.5 text-sm font-medium text-primary hover:bg-primary/10 transition-colors touch-manipulation"
                  >
                    <RefreshCw size={14} />
                    Reconnect
                  </button>
                )}
                <Link
                  to="/setup"
                  className="inline-flex items-center gap-2 rounded-full border border-border bg-card px-4 py-2.5 text-sm font-medium text-muted-foreground hover:text-foreground hover:border-border-strong transition-colors touch-manipulation"
                >
                  <Zap size={14} />
                  Setup guide
                </Link>
              </div>
            </div>
          </div>
        ) : threads.length === 0 && threadsQuery.isSuccess ? (
          <div className="flex h-full items-center justify-center px-4">
            <div className="text-center space-y-4 max-w-xs w-full">
              <div className="flex h-14 w-14 items-center justify-center rounded-full border border-border bg-muted mx-auto">
                <MessageSquare className="h-6 w-6 text-muted-foreground" />
              </div>
              <div className="space-y-1.5">
                <p className="text-sm font-semibold text-foreground">Start a conversation</p>
                <p className="text-xs text-muted-foreground leading-relaxed">
                  Create a new thread to begin.
                </p>
              </div>
              <button
                type="button"
                onClick={createThread}
                className="inline-flex items-center gap-2 rounded-full border border-primary/40 bg-primary/5 px-4 py-2.5 text-sm font-medium text-primary hover:bg-primary/10 transition-colors touch-manipulation"
              >
                <Plus size={14} />
                New thread
              </button>
            </div>
          </div>
        ) : (
          <div className="flex h-full items-center justify-center px-4">
            <div className="text-center space-y-3">
              <MessageSquare className="mx-auto h-8 w-8 text-muted-foreground" />
              <p className="text-sm text-muted-foreground">
                Select a thread or create a new one
              </p>
              <button
                type="button"
                onClick={() => setSheetOpen(true)}
                className="lg:hidden inline-flex items-center gap-2 rounded-full border border-border bg-card px-4 py-2.5 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors touch-manipulation"
              >
                <MessageSquare size={14} />
                View threads
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
