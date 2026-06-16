import { html } from "../../../lib/html.js";

const INITIALS = { user: "U", assistant: "IC", system: "S" };
const BG = {
  user: "border border-signal/30 bg-signal text-iron-950",
  assistant: "border border-white/10 bg-iron-700 text-iron-100",
  system: "bg-copper text-iron-950",
};

export function Avatar({ role, className = "" }) {
  return html`
    <div
      className=${[
        "flex h-7 w-7 shrink-0 items-center justify-center rounded-full font-mono text-[10px] font-semibold",
        BG[role] || BG.assistant,
        className,
      ].join(" ")}
    >
      ${INITIALS[role] || "IC"}
    </div>
  `;
}
