import type { UIMessage } from "@tanstack/ai-react";
import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import {
  AlertCircle,
  FileText,
  Globe,
  MessageSquare,
  Monitor,
  Plus,
  RefreshCw,
  ShieldCheck,
  Terminal,
  Trash2,
  Unplug,
  Wrench,
  Zap,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
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
import type { AttachmentLimits, StagedAttachment } from "@/lib/attachments";
import { useIronclawChat } from "@/hooks/use-ironclaw-chat";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";
import { useVerboseMode } from "@/hooks/use-verbose-mode";
import { messagesToUIMessages } from "@/lib/ironclaw-message-parts";

export const Route = createFileRoute("/_layout/")({
  component: ChatPage,
});

interface ThreadMeta {
  threadId: string;
  title: string | null;
  scope: { tenantId: string; agentId: string; projectId?: string };
  createdByActorId: string;
}

function toolIconForName(name: string) {
  const n = name.toLowerCase();
  if (n.includes("file") || n.includes("read") || n.includes("write") || n.includes("path"))
    return FileText;
  if (n.includes("web") || n.includes("search") || n.includes("fetch") || n.includes("http") || n.includes("url"))
    return Globe;
  if (n.includes("shell") || n.includes("bash") || n.includes("terminal") || n.includes("exec") || n.includes("code") || n.includes("run"))
    return Terminal;
  if (n.includes("browser") || n.includes("screenshot") || n.includes("page") || n.includes("click"))
    return Monitor;
  return Wrench;
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
  const apiClient = useApiClient();
  const chat = useIronclawChat(threadId, apiClient, initialMessages, verbose);
  const isBusy =
    chat.runState.phase === "submitted" || chat.runState.phase === "running" ||
    chat.runState.phase === "awaiting_approval" || chat.runState.phase === "auth_required";

  const handleSend = useCallback(
    (content: string, attachments?: StagedAttachment[]) => {
      if (!content.trim() || isBusy) return;
      chat.sendMessage(content, attachments);
    },
    [chat.sendMessage, isBusy],
  );

  const messages = chat.messages;
  const runState = chat.runState;
  const currentRunMessageId = runState.runId ? `assistant:${runState.runId}` : null;
  const currentRunHasText = currentRunMessageId
    ? messages.some(
        (m) => m.id === currentRunMessageId &&
          m.parts.some((p) => p.type === "text" && ((p as { content?: string }).content?.trim()?.length ?? 0) > 0),
      )
    : false;

  const showLoading = runState.phase !== "idle" && runState.phase !== "failed" && runState.phase !== "cancelled";
  const toolActive = runState.activeToolName && !currentRunHasText;
  const ProgressIcon = runState.phase === "awaiting_approval" || runState.phase === "auth_required"
    ? ShieldCheck : toolActive ? toolIconForName(runState.activeToolName!) : null;

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
      />
      {runState.phase === "awaiting_approval" ? (
        <div className="flex w-full items-center gap-2 border-b border-amber-500/20 bg-amber-500/5 px-4 py-2 text-xs text-amber-600">
          <ShieldCheck size={12} className="shrink-0" />
          <span className="flex-1">{runState.gateHeadline ?? "Approval required"}</span>
          <Button
            size="sm"
            variant="outline"
            className="h-7 border-amber-500/30 bg-amber-500/10 px-2.5 text-xs font-medium text-amber-600 hover:bg-amber-500/20"
            onClick={() => runState.gateRef && chat.resolveGate(runState.gateRef, true)}
          >
            Approve
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="h-7 border-amber-500/30 bg-amber-500/10 px-2.5 text-xs font-medium text-amber-600 hover:bg-amber-500/20"
            onClick={() => runState.gateRef && chat.resolveGate(runState.gateRef, false)}
          >
            Deny
          </Button>
        </div>
      ) : runState.phase === "auth_required" ? (
        <div className="flex w-full items-center gap-2 border-b border-blue-500/20 bg-blue-500/5 px-4 py-2 text-xs text-blue-600">
          <ShieldCheck size={12} className="shrink-0" />
          <span className="flex-1">{runState.authHeadline ?? "Authentication required"}</span>
          {runState.authUrl ? (
            <a href={runState.authUrl} target="_blank" rel="noreferrer" className="rounded border border-blue-500/30 bg-blue-500/10 px-2 py-0.5 font-medium hover:bg-blue-500/20 transition-colors">Authorize</a>
          ) : null}
          <button type="button" onClick={() => chat.retry()} className="rounded border border-blue-500/30 bg-blue-500/10 px-2 py-0.5 font-medium hover:bg-blue-500/20 transition-colors">Resume run</button>
        </div>
      ) : runState.phase === "failed" ? (
        <div className="flex w-full items-center gap-2 border-b border-destructive/20 bg-destructive/5 px-4 py-2 text-xs text-destructive">
          <AlertCircle size={12} className="shrink-0" />
          <span className="flex-1 truncate">{runState.message ?? "Run failed"}</span>
        </div>
      ) : runState.phase === "cancelled" ? (
        <div className="flex w-full items-center gap-2 border-b border-border bg-muted/30 px-4 py-2 text-xs text-muted-foreground"><span>Run was cancelled</span></div>
      ) : null}

      <ChatMessageList
        streamLoading={isBusy}
        empty={initialMessages.length === 0 && !isBusy}
        emptyMessage={threadMetaError ? "Failed to load thread" : "No messages yet. Send a message to start."}
      >
        {messages.filter((m) => m.parts.length > 0).map((message) => (
          <ChatMessage key={message.id} message={message} verbose={verbose} />
        ))}
        {showLoading ? (
          <div className="flex items-start gap-3">
            <div className="rounded-2xl rounded-bl-sm bg-muted px-4 py-2.5">
              <div className="flex items-center gap-2">
                {ProgressIcon ? <ProgressIcon size={11} className="text-muted-foreground/60 shrink-0" /> : null}
                <div className="flex items-center gap-1">
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/40 [animation-delay:0ms]" />
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/40 [animation-delay:150ms]" />
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground/40 [animation-delay:300ms]" />
                </div>
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
            attachmentCapabilities={attachmentCapabilities}
            verbose={verbose}
            onToggleVerbose={toggleVerbose}
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
              <div className="flex flex-col items-center gap-2">
                {connectionStatus === "disconnected" && (
                  <button
                    type="button"
                    onClick={() => refetchStatus()}
                    className="inline-flex items-center gap-2 rounded-full border border-primary/40 bg-primary/5 px-4 py-2 text-sm font-medium text-primary hover:bg-primary/10 transition-colors"
                  >
                    <RefreshCw size={14} />
                    Reconnect
                  </button>
                )}
                <Link
                  to="/setup"
                  className="inline-flex items-center gap-2 rounded-full border border-border bg-card px-4 py-2 text-sm font-medium text-muted-foreground hover:text-foreground hover:border-border-strong transition-colors"
                >
                  <Zap size={14} />
                  Setup guide
                </Link>
              </div>
            </div>
          </div>
        ) : threads.length === 0 && threadsQuery.isSuccess ? (
          <div className="flex h-full items-center justify-center">
            <div className="text-center space-y-4 max-w-xs px-4">
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
                className="inline-flex items-center gap-2 rounded-full border border-primary/40 bg-primary/5 px-4 py-2 text-sm font-medium text-primary hover:bg-primary/10 transition-colors"
              >
                <Plus size={14} />
                New thread
              </button>
            </div>
          </div>
        ) : (
          <div className="flex h-full items-center justify-center">
            <div className="text-center">
              <MessageSquare className="mx-auto h-8 w-8 text-muted-foreground" />
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
