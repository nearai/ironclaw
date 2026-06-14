import { Link } from "@tanstack/react-router";
import { RefreshCw, Settings } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";
import type { ThreadState } from "@/hooks/use-thread-state";

interface ChatIdentityBarProps {
  threadState: ThreadState | null;
  onRebuild: () => void;
  onToggleMeta: () => void;
  isRebuilding?: boolean;
}

export function ChatIdentityBar({
  threadState,
  onRebuild,
  onToggleMeta,
  isRebuilding,
}: ChatIdentityBarProps) {
  const { status: connectionStatus } = useIronclawStatus();

  const scope = threadState?.thread.scope;
  const isConnected = connectionStatus === "connected";

  return (
    <div className="sticky top-0 z-10 flex shrink-0 items-center justify-between gap-2 border-b border-border bg-card/95 px-4 py-2 backdrop-blur-sm">
      <div className="flex items-center gap-3 min-w-0">
        <div className="flex items-center gap-2">
          <span
            className={`h-2 w-2 rounded-full shrink-0 ${
              isConnected
                ? "bg-[color:var(--near-green)]"
                : "bg-destructive"
            }`}
          />
          <span className="text-xs text-muted-foreground hidden sm:inline">
            {isConnected ? "Connected" : "Disconnected"}
          </span>
        </div>
        {scope && (
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground min-w-0">
            <span className="hidden md:inline truncate">
              Tenant: {scope.tenantId || "-"}
            </span>
            <span className="hidden md:inline text-muted-foreground/40">|</span>
            <span className="truncate">Agent: {scope.agentId || "-"}</span>
            {scope.projectId && (
              <>
                <span className="text-muted-foreground/40 hidden md:inline">|</span>
                <span className="hidden md:inline truncate">Project: {scope.projectId}</span>
              </>
            )}
          </div>
        )}
      </div>
      <div className="flex items-center gap-1">
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
        <Link to="/ironclaw" className="flex items-center">
          <Button variant="ghost" size="icon" className="h-7 w-7" title="IronClaw settings">
            <Settings size={12} />
          </Button>
        </Link>
      </div>
    </div>
  );
}
