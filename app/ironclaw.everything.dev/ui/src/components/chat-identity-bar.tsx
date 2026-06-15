import { Link } from "@tanstack/react-router";
import { PanelLeft, RefreshCw, Settings } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";
import type { ThreadState } from "@/hooks/use-thread-state";

interface ChatIdentityBarProps {
  threadState: ThreadState | null;
  onRebuild: () => void;
  onToggleMeta: () => void;
  isRebuilding?: boolean;
  onOpenMobileSidebar?: () => void;
  onToggleDesktopSidebar?: () => void;
  activeThreadTitle?: string;
}

export function ChatIdentityBar({
  threadState,
  onRebuild,
  onToggleMeta,
  isRebuilding,
  onOpenMobileSidebar,
  onToggleDesktopSidebar,
  activeThreadTitle,
}: ChatIdentityBarProps) {
  const { status: connectionStatus } = useIronclawStatus();

  const scope = threadState?.thread.scope;
  const isConnected = connectionStatus === "connected";

  return (
    <div className="sticky top-0 z-10 flex shrink-0 items-center justify-between gap-2 border-b border-border bg-card/95 px-2 sm:px-3 py-2 backdrop-blur-sm">
      <div className="flex items-center gap-2 min-w-0">
        {onOpenMobileSidebar && (
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7 lg:hidden"
            onClick={onOpenMobileSidebar}
          >
            <PanelLeft size={14} />
          </Button>
        )}
        {onToggleDesktopSidebar && (
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7 hidden lg:flex"
            onClick={onToggleDesktopSidebar}
          >
            <PanelLeft size={14} />
          </Button>
        )}
        <span
          className={`h-1.5 w-1.5 rounded-full shrink-0 ${
            isConnected ? "bg-[color:var(--near-green)]" : "bg-destructive"
          }`}
        />
        {activeThreadTitle && (
          <span className="text-xs font-medium text-muted-foreground truncate">
            {activeThreadTitle}
          </span>
        )}
        {scope && (
          <div className="hidden md:flex items-center gap-1.5 text-xs text-muted-foreground min-w-0">
            <span className="text-muted-foreground/40 shrink-0">|</span>
            <span className="truncate">Tenant: {scope.tenantId || "-"}</span>
            <span className="text-muted-foreground/40 shrink-0">|</span>
            <span className="truncate">Agent: {scope.agentId || "-"}</span>
            {scope.projectId && (
              <>
                <span className="text-muted-foreground/40 shrink-0">|</span>
                <span className="truncate">Project: {scope.projectId}</span>
              </>
            )}
          </div>
        )}
      </div>
      <div className="flex items-center gap-1 shrink-0">
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={onToggleMeta}
          title="Thread info"
        >
          <span className="text-xs font-semibold text-muted-foreground">&#8862;</span>
        </Button>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          onClick={onRebuild}
          disabled={isRebuilding}
          title="Rebuild thread from server"
        >
          <RefreshCw size={12} className={isRebuilding ? "animate-spin" : ""} />
        </Button>
        <Link to="/setup" className="flex items-center">
          <Button variant="ghost" size="icon" className="h-7 w-7" title="IronClaw settings">
            <Settings size={12} />
          </Button>
        </Link>
      </div>
    </div>
  );
}
