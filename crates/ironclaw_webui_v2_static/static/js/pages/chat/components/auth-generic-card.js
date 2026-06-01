import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";

export function AuthGenericCard({ gate, onCancel }) {
  const t = useT();

  return html`
    <form
      className="mx-auto w-full max-w-lg rounded-xl border border-[rgba(76,167,230,0.34)] bg-[rgba(76,167,230,0.08)] p-4"
      onSubmit=${(e) => e.preventDefault()}
    >
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-[rgba(76,167,230,0.28)] bg-[rgba(76,167,230,0.1)] text-[#8fc8f2]">
          <${Icon} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">
          ${gate?.headline || t("authGate.title")}
        </span>
      </div>
      ${gate?.body &&
      html`<div className="mb-3 text-sm text-iron-200">${gate.body}</div>`}
      <div className="mb-3 text-sm text-iron-200">
        ${t("authGate.unsupportedChallenge")}
      </div>
      <div className="flex flex-wrap gap-2">
        <${Button}
          type="button"
          variant="secondary"
          onClick=${() => onCancel?.()}
        >
          ${t("authGate.cancel")}
        <//>
      </div>
    </form>
  `;
}
