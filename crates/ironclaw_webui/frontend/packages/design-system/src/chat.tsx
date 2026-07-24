/**
 * ChatMessage / AgentAvatar — the conversation surface.
 *
 * ChatMessage covers both turn shapes: `role="agent"` renders avatar +
 * open text (receipt cards and other components nest as children);
 * `role="user"` renders the right-aligned bubble. Copy inside follows
 * the Voice & copy rules (agent turns are receipts: past tense, the
 * reason, the escape hatch).
 */
import type { ReactNode } from "react";
import { Avatar, AvatarFallback } from "./avatar";
import { cn } from "./cn";
import { Icon } from "./icons";

/* ── AgentAvatar ──────────────────────────────────────────────────── */

export function AgentAvatar({ className = "" }: { className?: string }) {
  return (
    <Avatar className={cn("h-7 w-7", className)}>
      <AvatarFallback className="text-[var(--v2-accent-text)]">
        <Icon name="spark" className="h-3.5 w-3.5" />
      </AvatarFallback>
    </Avatar>
  );
}

/* ── ChatMessage ──────────────────────────────────────────────────── */

export interface ChatMessageProps {
  role: "agent" | "user";
  /** Override the leading avatar (agent turns only). */
  avatar?: ReactNode;
  className?: string;
  children?: ReactNode;
}

export function ChatMessage({ role, avatar, className = "", children }: ChatMessageProps) {
  if (role === "user") {
    return (
      <div className={cn("flex justify-end", className)}>
        <div className="max-w-[75%] rounded-[var(--v2-radius-lg)] bg-[var(--v2-surface-muted)] px-4 py-2.5 text-sm leading-6 text-[var(--v2-text-strong)]">
          {children}
        </div>
      </div>
    );
  }
  return (
    <div className={cn("flex gap-3", className)}>
      {avatar ?? <AgentAvatar />}
      <div className="min-w-0 max-w-[85%] space-y-3 text-sm leading-6 text-[var(--v2-text-muted)]">
        {children}
      </div>
    </div>
  );
}
