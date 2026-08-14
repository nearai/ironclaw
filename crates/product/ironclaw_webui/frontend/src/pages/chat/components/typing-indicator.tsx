import { NearProcessIndicator } from "./near-process-indicator";
import { useT } from "../../../lib/i18n";

type TypingIndicatorProps =
  | { state?: "working"; durationSeconds?: never }
  | { state: "done"; durationSeconds: number };

function formatDuration(durationSeconds: number): string {
  if (durationSeconds < 60) {
    return `${durationSeconds}s`;
  }

  const hours = Math.floor(durationSeconds / 3_600);
  const minutes = Math.floor((durationSeconds % 3_600) / 60);
  const seconds = durationSeconds % 60;

  return [hours, minutes, seconds]
    .map((part) => String(part).padStart(2, "0"))
    .join(":");
}

export function TypingIndicator({
  state = "working",
  durationSeconds,
}: TypingIndicatorProps = {}) {
  const t = useT();
  return (
    <div className="flex flex-col items-start">
      <div className="flex min-w-0 flex-col gap-2 v2-chat-readable-width">
        <div data-testid="typing-indicator" className="w-fit">
          <NearProcessIndicator
            state={state}
            label={
              state === "done"
                ? t("chat.workedFor", { duration: formatDuration(durationSeconds) })
                : t("chat.processWorking")
            }
          />
        </div>
      </div>
    </div>
  );
}
