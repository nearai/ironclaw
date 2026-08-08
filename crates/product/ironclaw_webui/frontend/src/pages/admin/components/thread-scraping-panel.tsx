// @ts-nocheck
import React from "react";
import { Button } from "../../../design-system/button";
import { Panel } from "../../../design-system/primitives";
import { useT } from "../../../lib/i18n";
import { useThreadScrape } from "../hooks/useThreadScrape";

// A single thread artifact can carry up to 1,000 messages / ~16MiB of text;
// rendering all of it into the DOM in one pass stalls the main thread. Render
// a bounded window and expose a "show more" expander instead.
const INITIAL_MESSAGE_WINDOW = 50;

export function ThreadScrapingPanel({ userId }) {
  const t = useT();
  const {
    threads,
    nextCursor,
    isLoading,
    isLoadingMore,
    isLoadingArtifact,
    selectedThreadId,
    selectThread,
    artifact,
    loadMore,
    downloadingRunId,
    downloadRun,
    downloadThreadArtifact,
    errorKey,
  } = useThreadScrape(userId);
  const [visibleMessageCount, setVisibleMessageCount] = React.useState(
    INITIAL_MESSAGE_WINDOW,
  );

  // A new selection (thread or target user) starts a fresh render window.
  React.useEffect(() => {
    setVisibleMessageCount(INITIAL_MESSAGE_WINDOW);
  }, [selectedThreadId]);

  const runIds = React.useMemo(
    () =>
      Array.from(
        new Set(
          (artifact?.messages || [])
            .map((message) => message.run_id)
            .filter(Boolean),
        ),
      ),
    [artifact],
  );

  return (
    <Panel className="p-5 sm:p-6" data-testid="admin-thread-scraping">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            {t("admin.threadScraping.title")}
          </h3>
          <p className="mt-2 text-sm text-iron-300">{t("admin.threadScraping.description")}</p>
        </div>
        {artifact && (
          <Button
            size="sm"
            variant="secondary"
            data-testid="admin-thread-scraping-download-thread"
            onClick={downloadThreadArtifact}
          >
            {t("admin.threadScraping.downloadThread")}
          </Button>
        )}
      </div>

      {errorKey && <p className="mt-4 text-sm text-red-200" role="alert">{t(errorKey)}</p>}
      {isLoading && <p className="mt-4 text-sm text-iron-300">{t("common.loading")}</p>}
      {!isLoading && !errorKey && threads.length === 0 && (
        <p className="mt-4 text-sm text-iron-300">{t("admin.threadScraping.empty")}</p>
      )}

      {threads.length > 0 && (
        <div className="mt-4 grid gap-4 lg:grid-cols-[minmax(0,18rem)_minmax(0,1fr)]">
          <div className="max-h-[32rem] space-y-2 overflow-y-auto pr-1">
            {threads.map((thread) => (
              <button
                key={thread.thread_id}
                type="button"
                aria-pressed={selectedThreadId === thread.thread_id}
                data-testid="admin-thread-scraping-thread"
                onClick={() => selectThread(thread.thread_id)}
                className={`w-full rounded-lg border px-3 py-2 text-left ${
                  selectedThreadId === thread.thread_id
                    ? "border-signal/45 bg-signal/10"
                    : "border-white/10 bg-white/[0.03] hover:border-white/20"
                }`}
              >
                <span className="block truncate text-sm text-iron-100">
                  {thread.title || t("admin.threadScraping.untitled")}
                </span>
                <span className="mt-1 block truncate font-mono text-[10px] text-iron-400">
                  {thread.thread_id}
                </span>
              </button>
            ))}
            {nextCursor && (
              <Button
                className="w-full"
                size="sm"
                variant="secondary"
                disabled={isLoadingMore}
                data-testid="admin-thread-scraping-load-more"
                onClick={loadMore}
              >
                {isLoadingMore ? t("common.loading") : t("common.loadMore")}
              </Button>
            )}
          </div>

          <div className="min-w-0">
            {isLoadingArtifact && <p className="text-sm text-iron-300">{t("common.loading")}</p>}
            {!isLoadingArtifact && !artifact && (
              <p className="text-sm text-iron-300">{t("admin.threadScraping.selectThread")}</p>
            )}
            {artifact && (
              <div className="space-y-3">
                {runIds.length > 0 && (
                  <div className="flex flex-wrap gap-2">
                    {runIds.map((runId) => (
                      <Button
                        key={runId}
                        size="sm"
                        variant="secondary"
                        disabled={Boolean(downloadingRunId)}
                        onClick={() => downloadRun(runId)}
                      >
                        {downloadingRunId === runId
                          ? t("common.loading")
                          : t("admin.threadScraping.downloadRun", { runId: String(runId).slice(0, 8) })}
                      </Button>
                    ))}
                  </div>
                )}
                <div className="max-h-[32rem] space-y-3 overflow-y-auto pr-1">
                  {(artifact.messages || []).slice(0, visibleMessageCount).map((message) => (
                    <div key={message.message_id} className="rounded-lg border border-white/10 bg-white/[0.03] p-3">
                      <div className="flex flex-wrap items-center gap-2 font-mono text-[10px] uppercase tracking-wide text-iron-400">
                        <span>{message.kind}</span>
                        {message.run_id && <span>{String(message.run_id).slice(0, 8)}</span>}
                      </div>
                      <div className="mt-2 whitespace-pre-wrap break-words text-sm text-iron-100">
                        {message.content}
                      </div>
                    </div>
                  ))}
                  {(artifact.messages || []).length > visibleMessageCount && (
                    <Button
                      className="w-full"
                      size="sm"
                      variant="secondary"
                      data-testid="admin-thread-scraping-show-more"
                      onClick={() =>
                        setVisibleMessageCount((current) =>
                          Math.min(
                            (artifact.messages || []).length,
                            current + INITIAL_MESSAGE_WINDOW,
                          ),
                        )
                      }
                    >
                      {t("common.loadMore")}
                    </Button>
                  )}
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </Panel>
  );
}
