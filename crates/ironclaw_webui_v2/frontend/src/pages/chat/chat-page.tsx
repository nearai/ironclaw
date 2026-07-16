// @ts-nocheck
import { useLocation, useNavigate, useOutletContext, useParams } from "react-router";
import React from "react";
import { Chat } from "./chat";
import { ConnectionStatus } from "./components/connection-status";

export function ChatPage() {
  const {
    threadsState,
    gatewayStatus,
    globalAutoApproveEnabled = false,
    setHeaderStatus,
  } = useOutletContext();
  const { threadId: urlThreadId } = useParams();
  const navigate = useNavigate();
  const location = useLocation();
  const composerDraft = location.state?.composerDraft || "";
  const routeThreadId = urlThreadId || null;

  const handleConnectionStatusChange = React.useCallback(
    (status) => setHeaderStatus(<ConnectionStatus status={status} />),
    [setHeaderStatus],
  );

  React.useEffect(
    () => () => setHeaderStatus(null),
    [setHeaderStatus],
  );

  React.useEffect(() => {
    if (routeThreadId && routeThreadId !== threadsState.activeThreadId) {
      threadsState.setActiveThreadId(routeThreadId);
    } else if (!routeThreadId) {
      threadsState.setActiveThreadId(null);
    }
  }, [routeThreadId]);

  const handleSelectThread = React.useCallback(
    (id, options = {}) => {
      if (!id) {
        threadsState.setActiveThreadId(null);
        navigate("/chat", options);
        return;
      }
      threadsState.setActiveThreadId(id);
      navigate(`/chat/${id}`, options);
    },
    [threadsState, navigate]
  );

  return (
    <Chat
      threads={threadsState.threads}
      activeThreadId={routeThreadId}
      onSelectThread={handleSelectThread}
      isCreatingThread={threadsState.isCreating}
      composerDraft={composerDraft}
      composerResetKey={location.key}
      gatewayStatus={gatewayStatus}
      globalAutoApproveEnabled={globalAutoApproveEnabled}
      onConnectionStatusChange={handleConnectionStatusChange}
    />
  );
}
