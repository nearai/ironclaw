import { NearProcessIndicator } from "./near-process-indicator";

export function TypingIndicator() {
  return (
    <div className="flex flex-col items-start">
      <div className="flex min-w-0 flex-col gap-2 v2-chat-readable-width">
        <div data-testid="typing-indicator" className="w-fit">
          <NearProcessIndicator state="working" label="Working…" />
        </div>
      </div>
    </div>
  );
}
