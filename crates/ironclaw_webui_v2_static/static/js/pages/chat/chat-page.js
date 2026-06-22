import { useLocation, useNavigate, useOutletContext, useParams } from "react-router";
import { React, html } from "../../lib/html.js";
import { Chat } from "./chat.js";

export function ChatPage() {
  const { threadsState, gatewayStatus } = useOutletContext();
  const { threadId: urlThreadId } = useParams();
  const navigate = useNavigate();
  const location = useLocation();
  const composerDraft = location.state?.composerDraft || "";
  const threads = threadsState.threads || [];
  const activeThreadId = threadsState.activeThreadId;
  const setActiveThreadId = threadsState.setActiveThreadId;
  const threadsLoaded = Boolean(threadsState.isLoaded);
  const threadsFetching = Boolean(threadsState.isFetching);
  const threadListSettled = threadsLoaded && !threadsFetching;
  const isReservedThreadRoute = urlThreadId === "newchat";
  const isMalformedThreadRoute = Boolean(urlThreadId && urlThreadId.trim() === "");
  const routeThreadSelectedLocally = Boolean(
    urlThreadId &&
      !isReservedThreadRoute &&
      !isMalformedThreadRoute &&
      activeThreadId === urlThreadId &&
      !threadListSettled
  );
  const routeThreadExistsInList = Boolean(
    urlThreadId &&
      !isReservedThreadRoute &&
      !isMalformedThreadRoute &&
      threadsLoaded &&
      threads.some((thread) => thread.id === urlThreadId)
  );
  const routeThreadCanOpen = routeThreadSelectedLocally || routeThreadExistsInList;
  const routeActiveThreadId = routeThreadCanOpen ? urlThreadId : null;

  React.useEffect(() => {
    if (!urlThreadId) {
      if (activeThreadId !== null) {
        setActiveThreadId(null);
      }
      return;
    }

    if (isReservedThreadRoute || isMalformedThreadRoute) {
      if (activeThreadId !== null) {
        setActiveThreadId(null);
      }
      navigate("/chat", { replace: true });
      return;
    }

    if (!threadListSettled && !routeThreadCanOpen) {
      return;
    }

    if (!routeThreadCanOpen) {
      if (activeThreadId !== null) {
        setActiveThreadId(null);
      }
      navigate("/chat", { replace: true });
      return;
    }

    if (urlThreadId !== activeThreadId) {
      setActiveThreadId(urlThreadId);
    }
  }, [
    activeThreadId,
    isMalformedThreadRoute,
    isReservedThreadRoute,
    navigate,
    routeThreadCanOpen,
    routeThreadSelectedLocally,
    setActiveThreadId,
    threadListSettled,
    urlThreadId,
  ]);

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

  return html`
    <${Chat}
      threads=${threads}
      activeThreadId=${routeActiveThreadId}
      onSelectThread=${handleSelectThread}
      isCreatingThread=${threadsState.isCreating}
      composerDraft=${composerDraft}
      composerResetKey=${location.key}
      gatewayStatus=${gatewayStatus}
    />
  `;
}
