import { Link } from "@tanstack/react-router";
import { Loader2, LogOut, RefreshCw, Unplug, Zap } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";

export function IronclawStatus() {
  const { status, refetch, disconnect } = useIronclawStatus();
  const [isDisconnecting, setIsDisconnecting] = useState(false);

  const handleDisconnect = async () => {
    setIsDisconnecting(true);
    try {
      await disconnect();
      toast.success("Disconnected from IronClaw");
    } catch {
      toast.error("Failed to disconnect");
    } finally {
      setIsDisconnecting(false);
    }
  };

  if (status === "checking") {
    return (
      <div className="flex h-8 items-center gap-1.5 rounded-full border border-border bg-card px-3 text-xs text-muted-foreground">
        <Loader2 size={10} className="animate-spin shrink-0" />
        <span className="hidden sm:inline">Connecting</span>
      </div>
    );
  }

  if (status === "never-connected") {
    return (
      <Link
        to="/ironclaw"
        className="flex h-8 items-center gap-1.5 rounded-full border border-primary/40 bg-primary/5 px-3 text-xs font-medium text-primary transition-colors hover:bg-primary/10 hover:border-primary/70"
      >
        <Zap size={10} className="shrink-0" />
        <span className="hidden sm:inline">Connect IronClaw</span>
        <span className="sm:hidden">Connect</span>
      </Link>
    );
  }

  if (status === "disconnected") {
    return (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="flex h-8 items-center gap-1.5 rounded-full border border-destructive/40 bg-destructive/5 px-3 text-xs font-medium text-destructive transition-colors hover:bg-destructive/10 hover:border-destructive/60 cursor-pointer"
          >
            <span className="h-1.5 w-1.5 rounded-full bg-destructive shrink-0" />
            <span className="hidden sm:inline">Reconnect</span>
            <span className="sm:hidden">
              <Unplug size={10} />
            </span>
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-52">
          <DropdownMenuLabel className="text-xs text-muted-foreground font-normal">
            IronClaw binary unreachable
          </DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onClick={refetch}
            className="gap-2 text-xs cursor-pointer"
          >
            <RefreshCw size={12} />
            Retry connection
          </DropdownMenuItem>
          <DropdownMenuItem asChild className="gap-2 text-xs cursor-pointer">
            <Link to="/ironclaw">
              <Zap size={12} />
              Setup guide
            </Link>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    );
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="flex h-8 items-center gap-1.5 rounded-full border border-[color:var(--near-green)]/40 bg-[color:var(--near-green)]/5 px-3 text-xs font-medium text-[color:var(--near-green)] transition-colors hover:bg-[color:var(--near-green)]/10 cursor-pointer"
        >
          <span className="h-1.5 w-1.5 rounded-full bg-[color:var(--near-green)] animate-pulse shrink-0" />
          <span className="hidden sm:inline">Connected</span>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-52">
        <DropdownMenuLabel className="text-xs text-muted-foreground font-normal">
          IronClaw binary connected
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onClick={refetch}
          className="gap-2 text-xs cursor-pointer"
        >
          <RefreshCw size={12} />
          Refresh status
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={handleDisconnect}
          disabled={isDisconnecting}
          variant="destructive"
          className="gap-2 text-xs cursor-pointer"
        >
          {isDisconnecting ? (
            <Loader2 size={12} className="animate-spin" />
          ) : (
            <LogOut size={12} />
          )}
          {isDisconnecting ? "Disconnecting…" : "Disconnect"}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
