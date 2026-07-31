import { NearProcessIndicator } from "./near-process-indicator";

export function TypingIndicator({
  state = "working",
  durationSeconds,
}: {
  state?: "working" | "done";
  durationSeconds?: number;
} = {}) {
  return (
    <div className="flex flex-col items-start">
      <div className="flex min-w-0 flex-col gap-2 v2-chat-readable-width">
        <div data-testid="typing-indicator" className="w-fit">
          <NearProcessIndicator
            state={state}
            label={
              state === "done" ? `Worked for ${durationSeconds}s` : "Working…"
            }
          />
        </div>
      </div>
    </div>
  );
}
