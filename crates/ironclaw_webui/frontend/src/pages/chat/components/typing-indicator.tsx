import { TypingIndicator as TypingIndicatorBubble } from "@ironclaw/design-system";

/* Chat-thread placement of the design-system TypingIndicator: aligned
   with the agent side of the stream at the shared readable width. */
export function TypingIndicator() {
  return (
    <div className="flex flex-col items-start">
      <div className="flex min-w-0 flex-col gap-2 v2-chat-readable-width">
        <TypingIndicatorBubble />
      </div>
    </div>
  );
}
