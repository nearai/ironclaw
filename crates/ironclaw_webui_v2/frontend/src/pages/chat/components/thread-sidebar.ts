import { Icon } from "../../../design-system/icons.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

function formatTime(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  const now = new Date();
  const isToday = d.toDateString() === now.toDateString();
  if (isToday)
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

export function ThreadSidebar({
  threads,
  activeThreadId,
  onSelect,
  onCreate,
  isCreating,
  compact = false,
}) {
  const t = useT();
  const canCreate = !(
    activeThreadId &&
    threads.some((t) => t.id === activeThreadId && (t.turn_count || 0) === 0)
  );
  const createDisabled = isCreating || !canCreate;

  if (compact) {
    return html`
      <div className="flex items-center gap-2">
        <button
          onClick=${onCreate}
          disabled=${createDisabled}
          className="v2-button h-9 shrink-0 rounded-md border border-signal/25 bg-signal/10 px-3 text-xs font-semibold text-signal disabled:opacity-50"
        >
          ${isCreating ? t("chat.creating") : t("chat.newThread")}
        </button>
        <select
          value=${activeThreadId || ""}
          onChange=${(event) => onSelect(event.target.value || null)}
          className="v2-select h-9 min-w-0 flex-1 rounded-md border border-white/10 bg-iron-900 px-3 text-sm text-white outline-none focus:border-signal/60"
        >
          <option value="">${t("chat.selectConversation")}</option>
          ${threads.map(
            (thread) => html`
              <option key=${thread.id} value=${thread.id}>
                ${thread.title || `Thread ${thread.id.slice(0, 8)}`}
              </option>
            `
          )}
        </select>
      </div>
    `;
  }

  return html`
    <div
      className="flex h-full flex-col border-r border-white/10 bg-iron-950/72 backdrop-blur-xl"
    >
      <div
        className="flex items-center justify-between border-b border-white/10 px-5 py-5"
      >
        <div>
          <span className="text-sm font-semibold text-white"
            >${t("chat.conversations")}</span
          >
          <p
            className="mt-1 font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300"
          >
            ${t("chat.threads", { count: threads.length })}
          </p>
        </div>
        <button
          onClick=${onCreate}
          disabled=${createDisabled}
          className="v2-button inline-flex h-8 items-center gap-1.5 rounded-md border border-signal/25 bg-signal/10 px-2 text-xs font-medium text-signal hover:bg-signal/15 disabled:opacity-50"
        >
          ${isCreating
            ? t("chat.creating")
            : html`<${Icon} name="plus" className="h-3.5 w-3.5" /> ${t(
                  "chat.newThread"
                )}`}
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-2">
        ${threads.length === 0 &&
        html`<div
          className="mx-2 mt-3 rounded-md border border-dashed border-white/12 px-4 py-7 text-left text-xs leading-5 text-iron-300"
        >
          ${t("chat.noConversations")}
        </div>`}
        ${threads.map((thread) => {
          const active = thread.id === activeThreadId;
          return html`
            <button
              key=${thread.id}
              onClick=${() => onSelect(thread.id)}
              className=${[
                "v2-button mb-1 flex w-full justify-start items-start flex-col gap-1 rounded-md border px-3 py-3 text-left",
                active
                  ? "border-signal/35 bg-signal/10"
                  : "border-transparent hover:border-white/10 hover:bg-white/[0.045]",
              ].join(" ")}
            >
              <div className="flex items-center gap-2">
                <span className="truncate max-w-[150px] text-sm font-medium text-iron-100">
                  ${thread.title || `Thread ${thread.id.slice(0, 8)}`}
                </span>
                ${thread.state === "Processing" &&
                html`<span
                  className="v2-breathing-dot ml-auto h-2 w-2 rounded-full bg-signal"
                />`}
              </div>
              <div
                className="flex items-center gap-2 font-mono text-[11px] text-iron-300"
              >
                <span
                  >${t("chat.turns", { count: thread.turn_count || 0 })}</span
                >
                <span>/</span>
                <span>${formatTime(thread.updated_at)}</span>
              </div>
            </button>
          `;
        })}
      </div>
    </div>
  `;
}
