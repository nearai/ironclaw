// @ts-nocheck
// App-root run-completion notifications (2026-08-13 design §5–§9): boots
// the orchestrator once per authenticated page, exposes the unread cache
// for the header badge/list, and reports the active thread for
// focused-thread suppression. The orchestrator module is lazily imported so
// the eager /chat bundle stays flat.

import React from "react";
import { useNavigate } from "react-router";
import { useT } from "../lib/i18n";
import {
  runCompletionSnapshot,
  subscribeRunCompletionStore,
} from "../lib/run-completions/store";

export function useRunCompletions({ enabled = true, activeThreadId = null } = {}) {
  const t = useT();
  const navigate = useNavigate();
  const snapshot = React.useSyncExternalStore(
    subscribeRunCompletionStore,
    runCompletionSnapshot,
    runCompletionSnapshot,
  );
  const clientRef = React.useRef(null);

  React.useEffect(() => {
    if (!enabled) return undefined;
    let cancelled = false;
    let stop = null;
    import("../lib/run-completions/client").then((module) => {
      if (cancelled) return;
      clientRef.current = module;
      stop = module.startRunCompletions({
        inAppMessage: (unreadForThread) =>
          unreadForThread > 1
            ? t("runCompletions.toastMany", { count: unreadForThread })
            : t("runCompletions.toastOne"),
        navigateToThread: (threadId) =>
          navigate(`/chat/${encodeURIComponent(threadId)}`),
      });
    });
    return () => {
      cancelled = true;
      if (stop) stop();
    };
    // The orchestrator is a page-lifetime singleton; navigate/t are stable
    // enough that re-running on their identity would only churn options.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled]);

  React.useEffect(() => {
    if (!enabled) return;
    const module = clientRef.current;
    if (module) {
      module.reportActiveThread(activeThreadId);
      if (activeThreadId) module.reportThreadViewed(activeThreadId);
      return;
    }
    import("../lib/run-completions/client").then((loaded) => {
      loaded.reportActiveThread(activeThreadId);
      if (activeThreadId) loaded.reportThreadViewed(activeThreadId);
    });
  }, [enabled, activeThreadId, snapshot.unreadCount]);

  // Returning focus to a thread tab is fresh read evidence (§9.3): the
  // rendered history is on screen again, so unread completions for the
  // thread settle without requiring a new event.
  React.useEffect(() => {
    if (!enabled || !activeThreadId) return undefined;
    const onFocus = () => {
      const module = clientRef.current;
      if (module) module.reportThreadViewed(activeThreadId);
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [enabled, activeThreadId]);

  const messages = React.useMemo(
    () =>
      snapshot.notices.map((notice) => ({
        id: `run-completion:${notice.notice_id}`,
        runId: notice.run_id,
        href: `/chat/${encodeURIComponent(notice.thread_id)}`,
        // The panel renders title/body/timeLabel; completion rows carry the
        // fixed generic copy only (no generated content).
        title:
          notice.unread_count_for_thread > 1
            ? t("runCompletions.listItemMany", {
                count: notice.unread_count_for_thread,
              })
            : t("runCompletions.listItemOne"),
        body: t("runCompletions.toastOne"),
        timeLabel: notice.completed_at
          ? new Date(notice.completed_at).toLocaleTimeString([], {
              hour: "2-digit",
              minute: "2-digit",
            })
          : null,
        read: false,
        timestamp: notice.completed_at || null,
        kind: "run-completion",
      })),
    [snapshot.notices, t],
  );

  return {
    unreadCount: snapshot.unreadCount,
    notices: snapshot.notices,
    messages,
  };
}
