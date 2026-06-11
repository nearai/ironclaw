import { html } from "../../../lib/html.js";
import { Avatar } from "./avatar.js";

export function TypingIndicator() {
  return html`
    <div className="flex gap-3">
      <${Avatar} role="assistant" />
      <div
        className="rounded-[18px] border border-white/10 bg-iron-800/60 px-4 py-3"
      >
        <div className="flex gap-1">
          <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
          <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
          <span className="v2-typing-dot h-2 w-2 rounded-full bg-iron-200" />
        </div>
      </div>
    </div>
  `;
}
