import { html } from "../../../lib/html.js";
import { Avatar } from "./avatar.js";
import { useT } from "../../../lib/i18n.js";

export function TypingIndicator() {
  const t = useT();
  return html`
    <div className="flex flex-col items-start">
      <div className="flex min-w-0 max-w-[85%] flex-col gap-2">
        <div className="flex items-center gap-2 px-1">
          <${Avatar} role="assistant" />
          <span className="text-xs font-medium text-[var(--v2-text-muted)]">
            ${t("chat.identityAssistant")}
          </span>
        </div>
        <div
          className="w-fit rounded-[18px] border border-white/10 bg-iron-800/60 px-4 py-3"
        >
          <div className="flex gap-1">
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
            <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
          </div>
        </div>
      </div>
    </div>
  `;
}
