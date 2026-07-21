
export function TypingIndicator() {
  return (
    <div className="flex flex-col items-start">
      <div className="flex min-w-0 flex-col gap-2 v2-chat-readable-width">
        <div
          data-testid="typing-indicator"
          className="w-fit rounded-[18px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3"
        >
          <div className="flex gap-1">
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-[var(--v2-text)]" />
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-[var(--v2-text)]" />
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-[var(--v2-text)]" />
          </div>
        </div>
      </div>
    </div>
  );
}
