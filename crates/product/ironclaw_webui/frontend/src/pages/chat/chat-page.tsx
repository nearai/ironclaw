// @ts-nocheck
import { useLocation, useNavigate, useOutletContext, useParams } from "react-router";
import React from "react";
import { Chat } from "./chat";
import { ConnectionStatus } from "./components/connection-status";
import { OnboardingPairingCard } from "./components/onboarding-pairing-card";
import { useConnectLinkLanding } from "./hooks/useConnectLinkLanding";

export function ChatPage() {
  const {
    threadsState,
    gatewayStatus,
    regressionArtifactExportEnabled = false,
    globalAutoApproveEnabled = false,
    pendingRenderedNotification = null,
    onNotificationRendered,
    setHeaderStatus,
  } = useOutletContext();
  const { threadId: urlThreadId } = useParams();
  const navigate = useNavigate();
  const location = useLocation();
  const composerDraft = location.state?.composerDraft || "";
  const routeThreadId = urlThreadId || null;
  const { connectLanding, startConnectLinkOAuth, dismissConnectLanding } =
    useConnectLinkLanding();

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
    <>
      {connectLanding &&
      (
        <OnboardingPairingCard
          onboarding={connectLanding}
          onConfigure={startConnectLinkOAuth}
          onCancel={dismissConnectLanding}
        />
      )}
      <Chat
        threads={threadsState.threads}
        activeThreadId={routeThreadId}
        onSelectThread={handleSelectThread}
        isCreatingThread={threadsState.isCreating}
        composerDraft={composerDraft}
        composerResetKey={location.key}
        gatewayStatus={gatewayStatus}
        regressionArtifactExportEnabled={regressionArtifactExportEnabled}
        globalAutoApproveEnabled={globalAutoApproveEnabled}
        pendingRenderedNotification={pendingRenderedNotification}
        onNotificationRendered={onNotificationRendered}
        onConnectionStatusChange={handleConnectionStatusChange}
      />
    </>
  );
}
