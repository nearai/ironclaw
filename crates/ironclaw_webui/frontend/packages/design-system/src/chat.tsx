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

export interface ChatMessageProps {
  role: "agent" | "user";
  /** Override the leading avatar (agent turns only). */
  avatar?: ReactNode;
  className?: string;
  children?: ReactNode;
}

/* ── TypingIndicator ──────────────────────────────────────────────── */

/**
 * The agent-is-working bubble: three dots on the ambient typing loop
 * (`.v2-typing-dot`, tokens.css — the one sanctioned ambient animation
 * in the chat surface; static under prefers-reduced-motion).
 */
export function TypingIndicator({ className = "" }: { className?: string }) {
  return (
    <div
      data-testid="typing-indicator"
      className={cn(
        "w-fit rounded-[var(--v2-radius-bubble)] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3",
        className
      )}
    >
      <div className="flex gap-1">
        <span className="v2-typing-dot h-2 w-2 rounded-full bg-[var(--v2-text)]" />
        <span className="v2-typing-dot h-2 w-2 rounded-full bg-[var(--v2-text)]" />
        <span className="v2-typing-dot h-2 w-2 rounded-full bg-[var(--v2-text)]" />
      </div>
    </div>
  );
}

/* ── SuggestionChip ───────────────────────────────────────────────── */

export interface SuggestionChipProps {
  onClick?: () => void;
  disabled?: boolean;
  className?: string;
  children?: ReactNode;
}

/**
 * Prompt-suggestion pill under the composer: quiet until hover, where it
 * takes the accent. Compose inside SuggestionChipRow.
 */
export function SuggestionChip({
  onClick,
  disabled = false,
  className = "",
  children,
}: SuggestionChipProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "inline-flex cursor-pointer rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]",
        "px-3 py-1.5 text-xs text-[var(--v2-text-strong)]",
        "transition-[border-color,color] duration-[var(--v2-duration-fast)] ease-[var(--v2-ease-standard)]",
        "hover:border-[var(--v2-accent)]/40 hover:text-[var(--v2-accent-text)]",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className
      )}
    >
      {children}
    </button>
  );
}

/** Wrapping row for SuggestionChips. */
export function SuggestionChipRow({
  className = "",
  children,
}: {
  className?: string;
  children?: ReactNode;
}) {
  return <div className={cn("flex flex-wrap gap-2", className)}>{children}</div>;
}

/* ── ChatMessage ──────────────────────────────────────────────────── */

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
