import { React } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

// TODO: gateway restart is a v1 capability (POSTs `/restart` as a
// chat command via `/api/chat/events`). v2 has no equivalent. Until a
// v2 admin/system endpoint lands, this hook is a coherent no-op: it
// returns the full interface `RestartBanner` consumes so the banner
// renders cleanly (a disabled button with a clear "unavailable" reason)
// instead of breaking on undefined fields. Wire the real restart call
// back into `confirmRestart` once the endpoint exists.
export function useGatewayRestart() {
  const t = useT();
  const [confirmOpen, setConfirmOpen] = React.useState(false);

  const openConfirm = React.useCallback(() => setConfirmOpen(true), []);
  const closeConfirm = React.useCallback(() => setConfirmOpen(false), []);
  const confirmRestart = React.useCallback(() => setConfirmOpen(false), []);

  return {
    restartEnabled: false,
    unavailableReason: t("settings.restartUnavailable"),
    isRestarting: false,
    progressLabel: "",
    error: null,
    message: null,
    confirmOpen,
    openConfirm,
    closeConfirm,
    confirmRestart,
  };
}
