import { html } from "../../../lib/html.js";

export function RecoveryNotice({ notice, onRecover }) {
  return html`
    <div className="mx-auto flex max-w-xl flex-wrap items-center justify-center gap-3 rounded-lg border border-copper/30 bg-copper/10 px-4 py-3 text-sm text-copper">
      <span>${notice.message}</span>
      ${notice.status !== "loading" &&
      html`
        <button
          type="button"
          onClick=${onRecover}
          className="rounded-md border border-copper/40 px-2.5 py-1 text-xs font-medium hover:bg-copper/10"
        >
          Reload history
        </button>
      `}
    </div>
  `;
}
