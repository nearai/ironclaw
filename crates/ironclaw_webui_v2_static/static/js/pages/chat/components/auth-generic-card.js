/**
 * AuthGenericCard — fallback for unsupported / unknown auth challenge kinds.
 *
 * Status Pill + Drawer presentation (AuthGateShell). The drawer explains the
 * step must be completed elsewhere and offers a cancel action.
 */
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { AuthGateShell } from "./auth-gate-shell.js";

export function AuthGenericCard({ gate, onCancel }) {
  const t = useT();

  return html`
    <${AuthGateShell}
      icon="lock"
      headline=${gate?.headline || t("authGate.title")}
      body=${gate?.body || ""}
    >
      <form onSubmit=${(event) => event.preventDefault()}>
        <div className="mb-3 text-sm text-iron-200">
          ${t("authGate.unsupportedChallenge")}
        </div>
        <div className="flex flex-wrap gap-2">
          <${Button} type="button" variant="secondary" onClick=${() => onCancel?.()}>
            ${t("authGate.cancel")}
          <//>
        </div>
      </form>
    <//>
  `;
}
