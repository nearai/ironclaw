import { html } from "../../../lib/html.js";

export function TypingIndicator() {
  return html`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-700 font-mono text-[11px] font-semibold text-iron-100"
      >
        IC
      </div>
      <div
        className="rounded-[18px] border border-white/10 bg-iron-800/60 px-4 py-3"
      >
        <div className="flex gap-1">
          <span
            className="h-2 w-2 animate-bounce rounded-full bg-iron-200"
            style=${{ animationDelay: "0ms" }}
          />
          <span
            className="h-2 w-2 animate-bounce rounded-full bg-iron-200"
            style=${{ animationDelay: "150ms" }}
          />
          <span
            className="h-2 w-2 animate-bounce rounded-full bg-iron-200"
            style=${{ animationDelay: "300ms" }}
          />
        </div>
      </div>
    </div>
  `;
}
