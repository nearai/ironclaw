import { React } from "../../../lib/html.js";
import { readStoredToken, sendMessage } from "../../../lib/api.js";

const RESTART_TIMEOUT_MS = 60_000;
const RESTART_POLL_MS = 1_500;

export function useGatewayRestart({ gatewayStatus, gatewayStatusQuery }) {
  const [confirmOpen, setConfirmOpen] = React.useState(false);
  const [isRestarting, setIsRestarting] = React.useState(false);
  const [sentThreadId, setSentThreadId] = React.useState(null);
  const [sawDisconnect, setSawDisconnect] = React.useState(false);
  const [startedAt, setStartedAt] = React.useState(null);
  const [error, setError] = React.useState("");
  const [message, setMessage] = React.useState("");

  const restartEnabled = Boolean(gatewayStatus?.restart_enabled);
  const unavailableReason = gatewayStatus
    ? "Restart is only available for Docker gateway deployments."
    : "Gateway status is unavailable.";

  const openConfirm = React.useCallback(() => {
    setError("");
    setMessage("");
    setConfirmOpen(true);
  }, []);

  const closeConfirm = React.useCallback(() => {
    if (!isRestarting) setConfirmOpen(false);
  }, [isRestarting]);

  const confirmRestart = React.useCallback(async () => {
    if (!restartEnabled) {
      setError(unavailableReason);
      setConfirmOpen(false);
      return;
    }

    setError("");
    setMessage("");
    setSawDisconnect(false);
    setStartedAt(Date.now());
    setIsRestarting(true);

    try {
      const response = await sendMessage({ content: "/restart" });
      setSentThreadId(response?.thread_id || null);
      setConfirmOpen(false);
      gatewayStatusQuery?.refetch?.();
    } catch (err) {
      setError(err.message || "Restart request failed.");
      setIsRestarting(false);
      setStartedAt(null);
      setConfirmOpen(false);
    }
  }, [gatewayStatusQuery, restartEnabled, unavailableReason]);

  React.useEffect(() => {
    if (!isRestarting) return undefined;
    const timer = setInterval(() => {
      gatewayStatusQuery?.refetch?.();
    }, RESTART_POLL_MS);
    return () => clearInterval(timer);
  }, [gatewayStatusQuery, isRestarting]);

  React.useEffect(() => {
    if (!isRestarting) return;
    if (gatewayStatusQuery?.isError) {
      setSawDisconnect(true);
      return;
    }
    if (sawDisconnect && gatewayStatusQuery?.isSuccess) {
      setIsRestarting(false);
      setStartedAt(null);
      setSawDisconnect(false);
      setMessage("Gateway reconnected.");
    }
  }, [gatewayStatusQuery?.isError, gatewayStatusQuery?.isSuccess, isRestarting, sawDisconnect]);

  React.useEffect(() => {
    if (!isRestarting || !startedAt) return undefined;
    const timer = setTimeout(() => {
      setError("Restart did not reconnect within 60 seconds. Check the gateway logs.");
      setIsRestarting(false);
      setStartedAt(null);
    }, RESTART_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, [isRestarting, startedAt]);

  React.useEffect(() => {
    if (!isRestarting || !sentThreadId) return undefined;

    const token = readStoredToken();
    const url = token
      ? `/api/chat/events?token=${encodeURIComponent(token)}`
      : "/api/chat/events";
    const events = new EventSource(url);

    const fail = (detail) => {
      setError(detail || "Restart was rejected.");
      setIsRestarting(false);
      setStartedAt(null);
      setSawDisconnect(false);
    };

    const onResponse = (event) => {
      const data = parseEvent(event);
      if (!matchesThread(data, sentThreadId)) return;
      const content = String(data?.content || "").toLowerCase();
      if (
        content.includes("restart is not available") ||
        content.includes("restart is only available") ||
        content.includes("restart failed")
      ) {
        fail(data.content);
      }
    };

    const onToolCompleted = (event) => {
      const data = parseEvent(event);
      if (
        !matchesThread(data, sentThreadId) ||
        String(data?.name || "").toLowerCase() !== "restart"
      ) {
        return;
      }
      if (data.success === false) {
        fail(data.error || "Restart failed.");
      }
    };

    const onErrorEvent = (event) => {
      const data = parseEvent(event);
      if (matchesThread(data, sentThreadId)) {
        fail(data.message || data.error || "Restart failed.");
      }
    };

    events.addEventListener("response", onResponse);
    events.addEventListener("tool_completed", onToolCompleted);
    events.addEventListener("error", onErrorEvent);
    events.onerror = () => setSawDisconnect(true);

    return () => events.close();
  }, [isRestarting, sentThreadId]);

  const progressLabel = sawDisconnect
    ? "Gateway is restarting. Waiting for it to reconnect..."
    : "Restart request accepted. Waiting for the gateway to drop...";

  return {
    confirmOpen,
    openConfirm,
    closeConfirm,
    confirmRestart,
    isRestarting,
    restartEnabled,
    unavailableReason,
    progressLabel,
    error,
    message,
  };
}

function parseEvent(event) {
  try {
    return JSON.parse(event.data);
  } catch {
    return {};
  }
}

function matchesThread(data, threadId) {
  return data?.thread_id === threadId;
}
