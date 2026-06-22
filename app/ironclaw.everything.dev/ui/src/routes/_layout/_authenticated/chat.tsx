import { createFileRoute, Link, Outlet, useLocation, useNavigate, useMatchRoute } from "@tanstack/react-router";
import {
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  MessageSquare,
  Plus,
  Search,
  Trash2,
  Unplug,
  Zap,
} from "lucide-react";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useApiClient } from "@/app";
import { SubagentRow } from "@/components/thread-sidebar-row";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { type ConversationThread, useConversationThreads } from "@/hooks/use-conversation";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";
import { threadChatManager } from "@/hooks/use-thread-chat-manager";

export const Route = createFileRoute("/_layout/_authenticated/chat")({
  loader: async ({ context }) => {
    const { threadListQueryOptions } = await import("@/hooks/use-conversation");
    await context.queryClient.ensureQueryData(
      threadListQueryOptions(context.apiClient),
    );
  },
  component: ChatLayout,
});

const SIDEBAR_MIN_WIDTH = 200;
const SIDEBAR_MAX_WIDTH = 480;
const SIDEBAR_DEFAULT_WIDTH = 272;
const SIDEBAR_COLLAPSED_WIDTH = 40;

interface ChatLayoutContextValue {
  onOpenMobileSidebar: () => void;
  onToggleDesktopSidebar: () => void;
}

const ChatLayoutCtx = createContext<ChatLayoutContextValue>({
  onOpenMobileSidebar: () => {},
  onToggleDesktopSidebar: () => {},
});

export function useChatLayout() {
  return useContext(ChatLayoutCtx);
}

function ChatLayout() {
  const apiClient = useApiClient();
  const { status: connectionStatus } = useIronclawStatus();
  const navigate = useNavigate();
  const location = useLocation();
  const matchRoute = useMatchRoute();
  const threadMatch = matchRoute({ to: "/chat/$threadId" });
  const activeThreadId = threadMatch && typeof threadMatch === "object" && "params" in threadMatch
    ? (threadMatch as { params: { threadId: string } }).params.threadId
    : null;

  useEffect(() => {
    if (connectionStatus === "never-connected") {
      navigate({ to: "/setup", replace: true });
    }
  }, [connectionStatus, navigate]);

  const isDisconnected = connectionStatus === "disconnected" || connectionStatus === "never-connected";

  const threadsQuery = useConversationThreads();

  const threads = useMemo(() => {
    const all = (threadsQuery.data?.threads ?? []) as ConversationThread[];
    return all.sort((a, b) => {
      const aTime = a.updatedAt ?? a.createdAt ?? "";
      const bTime = b.updatedAt ?? b.createdAt ?? "";
      return bTime.localeCompare(aTime);
    });
  }, [threadsQuery.data]);

  const createThread = useCallback(async () => {
    try {
      const result = await apiClient.ironclaw.threads.create({
        clientActionId: `ui-${crypto.randomUUID()}`,
      });
      threadsQuery.refetch();
      navigate({ to: "/chat/$threadId", params: { threadId: result.threadId } });
    } catch {
      toast.error("Failed to create thread");
    }
  }, [apiClient, navigate, threadsQuery]);

  const [deleteConfirmTarget, setDeleteConfirmTarget] = useState<string | null>(null);

  const deleteThread = useCallback(
    async (threadId: string) => {
      setDeleteConfirmTarget(threadId);
    },
    [],
  );

  const confirmDelete = useCallback(async () => {
    const threadId = deleteConfirmTarget;
    if (!threadId) return;
    setDeleteConfirmTarget(null);
    try {
      await apiClient.ironclaw.threads.delete({ id: threadId });
      threadsQuery.refetch();
      if (activeThreadId === threadId) {
        navigate({ to: "/chat" });
      }
    } catch {
      toast.error("Failed to delete thread");
    }
  }, [deleteConfirmTarget, apiClient, activeThreadId, threadsQuery, navigate]);

  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [sidebarWidth, setSidebarWidth] = useState(SIDEBAR_DEFAULT_WIDTH);
  const [sheetOpen, setSheetOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");

  const filteredThreads = useMemo(() => {
    if (!searchQuery.trim()) return threads;
    const q = searchQuery.toLowerCase();
    return threads.filter((t) => {
      const title = (t.title ?? `Thread ${t.threadId.slice(0, 8)}`).toLowerCase();
      return title.includes(q);
    });
  }, [threads, searchQuery]);

  const threadState = useMemo(() => {
    const map = new Map<string, "running" | "needs-attention">();
    for (const thread of threads) {
      const session = threadChatManager.get(thread.threadId);
      if (!session) continue;
      if (session.pendingApprovals.length > 0) {
        map.set(thread.threadId, "needs-attention");
      } else if (session.isLoading || session.runId) {
        map.set(thread.threadId, "running");
      }
    }
    return map;
  }, [threads]);

  useEffect(() => {
    setSheetOpen(false);
  }, [location.pathname]);

  const [, tick] = useState(0);
  useEffect(() => {
    const unsubs = threads.map((t) =>
      threadChatManager.subscribe(t.threadId, () => tick((n) => n + 1)),
    );
    return () => unsubs.forEach((fn) => fn());
  }, [threads]);

  const isResizing = useRef(false);
  const startX = useRef(0);
  const startWidth = useRef(0);

  const handleResizeStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      isResizing.current = true;
      startX.current = e.clientX;
      startWidth.current = sidebarWidth;

      const onMouseMove = (ev: MouseEvent) => {
        if (!isResizing.current) return;
        const delta = ev.clientX - startX.current;
        const newWidth = Math.min(
          SIDEBAR_MAX_WIDTH,
          Math.max(SIDEBAR_MIN_WIDTH, startWidth.current + delta),
        );
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
    },
    [sidebarWidth, sidebarOpen],
  );

  const [expandedParents, setExpandedParents] = useState<Set<string>>(() => new Set());

  const subagentMap = useMemo(() => {
    const map = new Map<string, ConversationThread[]>();
    for (const thread of threads) {
      if (thread.isSubagent && thread.parentThreadId) {
        const children = map.get(thread.parentThreadId) ?? [];
        children.push(thread);
        map.set(thread.parentThreadId, children);
      }
    }
    return map;
  }, [threads]);

  const rootThreads = useMemo(
    () => filteredThreads.filter((t) => !t.isSubagent),
    [filteredThreads],
  );

  const orphanedSubagents = useMemo(() => {
    const parentIds = new Set(filteredThreads.map((t) => t.threadId));
    return filteredThreads.filter((t) => t.isSubagent && (!t.parentThreadId || !parentIds.has(t.parentThreadId)));
  }, [filteredThreads]);

  const toggleParent = useCallback((threadId: string) => {
    setExpandedParents((prev) => {
      const next = new Set(prev);
      if (next.has(threadId)) {
        next.delete(threadId);
      } else {
        next.add(threadId);
      }
      return next;
    });
  }, []);

  const statusDotClass =
    connectionStatus === "connected"
      ? "bg-[color:var(--near-green)]"
      : connectionStatus === "checking"
        ? "bg-muted-foreground animate-pulse"
        : "bg-destructive";

  const linkBase = "group flex min-w-0 w-full items-center gap-2 rounded-lg transition-colors cursor-pointer touch-manipulation relative text-muted-foreground hover:bg-muted active:bg-muted";

  const threadLinkClass = `${linkBase} px-3 py-2.5 text-left text-sm`;
  const subagentLinkClass = `${linkBase} px-3 py-1.5 text-left text-xs`;
  const indentClass = `pl-7`;

  const threadListContent = (
    <div className="flex flex-col min-h-0">
      {!isDisconnected && (
        <div className="px-2 pt-1.5 pb-1">
          <div className="relative">
            <Search size={12} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground/50" />
            <Input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search threads..."
              className="h-8 pl-7 pr-2 text-xs"
            />
          </div>
        </div>
      )}
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
          rootThreads.map((thread) => {
            const children = subagentMap.get(thread.threadId) ?? [];
            const isExpanded = expandedParents.has(thread.threadId);
            const hasChildren = children.length > 0;
            return (
              <div key={thread.threadId} className="relative">
                <Link
                  to="/chat/$threadId"
                  params={{ threadId: thread.threadId }}
                  className={threadLinkClass}
                  activeProps={{ className: "bg-primary/15 text-foreground" }}
                >
                  <span className="absolute left-px top-1/2 -translate-y-1/2 h-6 w-[3px] rounded-full group-data-[status=active]:bg-primary" />
                  <div className="flex items-center gap-1.5 min-w-0 flex-1">
                    {hasChildren && (
                      <button
                        type="button"
                        onClick={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          toggleParent(thread.threadId);
                        }}
                        className="shrink-0 p-0.5 hover:text-foreground transition-colors touch-manipulation"
                        aria-label={isExpanded ? "Collapse sub-agents" : "Expand sub-agents"}
                      >
                        <ChevronDown
                          size={12}
                          className={`transition-transform ${isExpanded ? "" : "-rotate-90"}`}
                        />
                      </button>
                    )}
                    <span className="relative shrink-0">
                      {threadState.get(thread.threadId) === "running" && (
                        <span className="absolute -right-0.5 -top-0.5 h-2 w-2 animate-pulse rounded-full bg-[color:var(--near-green)]" />
                      )}
                      {threadState.get(thread.threadId) === "needs-attention" && (
                        <span className="absolute -right-0.5 -top-0.5 h-2 w-2 rounded-full bg-amber-500" />
                      )}
                      <MessageSquare size={14} />
                    </span>
                    <span className="truncate text-xs">
                      {thread.title ?? `Thread ${thread.threadId.slice(0, 8)}`}
                    </span>
                    {hasChildren && (
                      <span className="shrink-0 rounded-full bg-muted-foreground/10 px-1.5 text-[10px] font-medium text-muted-foreground/60">
                        {children.length}
                      </span>
                    )}
                  </div>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      deleteThread(thread.threadId);
                    }}
                    className="shrink-0 p-1 -m-1 opacity-0 group-hover:opacity-100 focus:opacity-100 transition-opacity touch-manipulation"
                    aria-label="Delete thread"
                  >
                    <Trash2
                      size={12}
                      className="text-muted-foreground/40 hover:text-destructive transition-colors"
                    />
                  </button>
                </Link>
                {hasChildren && isExpanded && (
                  <div className="ml-4 border-l border-border/50 pl-1">
                    {children.map((child) => {
                      return (
                        <Link
                          key={child.threadId}
                          to="/chat/$threadId"
                          params={{ threadId: child.threadId }}
                          className={subagentLinkClass}
                          activeProps={{ className: "bg-primary/15 text-foreground" }}
                        >
                          <span className="absolute left-px top-1/2 -translate-y-1/2 h-5 w-[3px] rounded-full group-data-[status=active]:bg-primary" />
                          <SubagentRow thread={child} onDelete={deleteThread} />
                        </Link>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })
        )}
        {orphanedSubagents.length > 0 && (
          <div className="pt-2">
            <div className="px-3 py-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground/50">
              Orphaned sub-agents
            </div>
            {orphanedSubagents.map((child) => {
              return (
                <Link
                  key={child.threadId}
                  to="/chat/$threadId"
                  params={{ threadId: child.threadId }}
                  className={`${subagentLinkClass} ${indentClass}`}
                  activeProps={{ className: "bg-primary/15 text-foreground" }}
                >
                  <span className="absolute left-px top-1/2 -translate-y-1/2 h-5 w-[3px] rounded-full group-data-[status=active]:bg-primary" />
                  <SubagentRow thread={child} onDelete={deleteThread} />
                </Link>
              );
            })}
          </div>
        )}
      </div>
    </ScrollArea>
    </div>
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

  const ctx: ChatLayoutContextValue = {
    onOpenMobileSidebar: () => setSheetOpen(true),
    onToggleDesktopSidebar: () => setSidebarOpen((v) => !v),
  };

  return (
    <ChatLayoutCtx.Provider value={ctx}>
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
              {rootThreads.slice(0, 8).map((thread) => (
                <button
                  key={thread.threadId}
                  type="button"
                  onClick={() => {
                    navigate({ to: "/chat/$threadId", params: { threadId: thread.threadId } });
                    setSidebarOpen(true);
                  }}
                  className={`w-8 h-8 rounded-lg flex items-center justify-center transition-colors touch-manipulation ${
                    activeThreadId === thread.threadId
                      ? "bg-primary/15 text-foreground"
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
          <Outlet />
        </div>
      </div>

      <Dialog open={deleteConfirmTarget !== null} onOpenChange={(open) => { if (!open) setDeleteConfirmTarget(null); }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete thread?</DialogTitle>
            <DialogDescription>
              This will permanently delete this thread and all its messages. This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="secondary" onClick={() => setDeleteConfirmTarget(null)}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={confirmDelete}>
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </ChatLayoutCtx.Provider>
  );
}
