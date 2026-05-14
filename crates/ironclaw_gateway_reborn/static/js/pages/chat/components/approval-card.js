import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";

export function ApprovalCard({ gate, onApprove, onDeny, onAlways }) {
  const t = useT();
  const { toolName, description, parameters, allowAlways } = gate;

  return html`
    <div className="mx-auto max-w-lg rounded-xl border border-copper/30 bg-copper/10 p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-copper/25 bg-copper/10 text-copper">
          <${Icon} name="lock" className="h-4 w-4" />
        </span>
        <span className="font-semibold text-white">${t("approval.title")}</span>
      </div>
      <div className="mb-1 text-sm font-medium text-iron-100">${toolName}</div>
      <div className="mb-3 text-sm text-iron-200">${description}</div>
      ${parameters && html`<pre className="mb-3 overflow-x-auto rounded-md bg-iron-950 p-2 font-mono text-xs text-iron-100">${parameters}</pre>`}
      <div className="flex flex-wrap gap-2">
        <${Button} variant="primary" onClick=${() => onApprove()}>${t("approval.approve")}<//>
        <${Button} variant="secondary" onClick=${() => onDeny()}>${t("approval.deny")}<//>
        ${allowAlways && html`<${Button} variant="ghost" onClick=${() => onAlways()}>${t("approval.always")}<//>`}
      </div>
    </div>
  `;
}
