import { React } from "../../../lib/html.js";

// TODO: gateway restart is a v1 capability (POSTs `/restart` as a
// chat command via `/api/chat/events`). v2 has no equivalent.
// This hook becomes a no-op so the settings Restart Banner renders
// without hitting any v1 path. When the v2 admin/system endpoint
// lands, wire it back here.
export function useGatewayRestart() {
  const [progress, setProgress] = React.useState(null);
  const [error, setError] = React.useState(null);

  const restart = React.useCallback(async () => {
    setError("TODO: requires v2 system endpoint");
  }, []);

  return {
    restart,
    progress,
    error,
    isRestarting: false,
  };
}
