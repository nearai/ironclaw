/**
 * AuthGenericCard — fallback for unsupported / unknown auth challenge kinds.
 *
 * Status Pill + Drawer presentation (AuthGateShell). The drawer explains the
 * step must be completed elsewhere and offers a cancel action.
 */
import { useT } from "../../../lib/i18n";
import { Button } from "../../../design-system/button";
import { AuthGateShell } from "./auth-gate-shell";

export function AuthGenericCard({ gate, onCancel }) {
  const t = useT();

  return (
    <AuthGateShell
      icon="lock"
      headline={gate?.headline || t("authGate.title")}
      body={gate?.body || ""}
      challengeKind="other"
    >
      <form onSubmit={(event) => event.preventDefault()}>
        <div className="mb-3 text-sm text-iron-200">
          {t("authGate.unsupportedChallengeNeutral")}
        </div>
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="secondary" onClick={() => onCancel?.()}>
            {t("authGate.cancel")}
          </Button>
        </div>
      </form>
    </AuthGateShell>
  );
}
