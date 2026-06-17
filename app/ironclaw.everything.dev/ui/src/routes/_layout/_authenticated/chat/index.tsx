import { createFileRoute, Link } from "@tanstack/react-router";
import { MessageSquare, Unplug, Zap } from "lucide-react";
import { useConversationThreads } from "@/hooks/use-conversation";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";

export const Route = createFileRoute("/_layout/_authenticated/chat/")({
  component: ChatIndex,
});

function ChatIndex() {
  const { status: connectionStatus } = useIronclawStatus();
  const threadsQuery = useConversationThreads();
  const isDisconnected = connectionStatus === "disconnected" || connectionStatus === "never-connected";
  const threads = threadsQuery.data?.threads ?? [];

  if (isDisconnected) {
    return (
      <div className="flex h-full items-center justify-center px-4">
        <div className="text-center space-y-4 max-w-xs w-full">
          <div className="flex h-14 w-14 items-center justify-center rounded-full border border-border bg-muted mx-auto">
            <Unplug className="h-6 w-6 text-muted-foreground" />
          </div>
          <div className="space-y-1.5">
            <p className="text-sm font-semibold text-foreground">
              {connectionStatus === "never-connected" ? "IronClaw not connected" : "Connection lost"}
            </p>
            <p className="text-xs text-muted-foreground leading-relaxed">
              {connectionStatus === "never-connected"
                ? "Run the IronClaw binary locally, then return here to start chatting."
                : "The IronClaw binary stopped responding. Check that it's still running."}
            </p>
          </div>
          <Link
            to="/setup"
            className="inline-flex items-center gap-2 rounded-full border border-border bg-card px-4 py-2.5 text-sm font-medium text-muted-foreground hover:text-foreground hover:border-border-strong transition-colors touch-manipulation"
          >
            <Zap size={14} />
            Setup guide
          </Link>
        </div>
      </div>
    );
  }

  if (threads.length === 0 && threadsQuery.isSuccess) {
    return (
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
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full items-center justify-center px-4">
      <div className="text-center space-y-3">
        <MessageSquare className="mx-auto h-8 w-8 text-muted-foreground" />
        <p className="text-sm text-muted-foreground">Select a thread or create a new one</p>
      </div>
    </div>
  );
}
