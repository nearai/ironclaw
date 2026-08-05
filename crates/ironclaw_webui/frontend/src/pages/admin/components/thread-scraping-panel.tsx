// @ts-nocheck
import React from "react";
import { Button } from "../../../design-system/button";
import { Panel } from "../../../design-system/primitives";
import { saveBlob } from "../../../lib/download";
import { useT } from "../../../lib/i18n";
import {
  fetchThreadScrapeArtifact,
  fetchThreadScrapeRunArtifact,
  fetchThreadScrapeThreads,
} from "../lib/admin-api";

function artifactFilename(prefix, id) {
  const safeId = String(id || "artifact").replace(/[^a-zA-Z0-9._-]/g, "_");
  return `ironclaw-${prefix}-${safeId}.json`;
}

function saveArtifact(artifact, filename) {
  saveBlob(
    new Blob([`${JSON.stringify(artifact, null, 2)}\n`], {
      type: "application/json",
    }),
    filename,
  );
}

export function ThreadScrapingPanel({ userId }) {
  const t = useT();
  const [threads, setThreads] = React.useState([]);
  const [selectedThreadId, setSelectedThreadId] = React.useState("");
  const [artifact, setArtifact] = React.useState(null);
  const [isLoading, setIsLoading] = React.useState(true);
  const [isLoadingArtifact, setIsLoadingArtifact] = React.useState(false);
  const [downloadingRunId, setDownloadingRunId] = React.useState("");
  const [error, setError] = React.useState("");

  React.useEffect(() => {
    const controller = new AbortController();
    setIsLoading(true);
    setError("");
    fetchThreadScrapeThreads(userId, { limit: 100, signal: controller.signal })
      .then((response) => setThreads(Array.isArray(response?.threads) ? response.threads : []))
      .catch((requestError) => {
        if (requestError?.name !== "AbortError") {
          setError(requestError instanceof Error ? requestError.message : t("admin.threadScraping.loadFailed"));
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) setIsLoading(false);
      });
    return () => controller.abort();
  }, [t, userId]);

  const selectThread = async (threadId) => {
    setSelectedThreadId(threadId);
    setArtifact(null);
    setError("");
    setIsLoadingArtifact(true);
    try {
      setArtifact(await fetchThreadScrapeArtifact(userId, threadId));
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : t("admin.threadScraping.loadFailed"));
    } finally {
      setIsLoadingArtifact(false);
    }
  };

  const downloadRun = async (runId) => {
    if (!selectedThreadId || !runId || downloadingRunId) return;
    setDownloadingRunId(runId);
    setError("");
    try {
      const runArtifact = await fetchThreadScrapeRunArtifact(userId, selectedThreadId, runId);
      saveArtifact(runArtifact, artifactFilename("run", runId));
    } catch (requestError) {
      setError(requestError instanceof Error ? requestError.message : t("admin.threadScraping.downloadFailed"));
    } finally {
      setDownloadingRunId("");
    }
  };

  const runIds = React.useMemo(
    () => Array.from(new Set((artifact?.messages || []).map((message) => message.run_id).filter(Boolean))),
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
            onClick={() => saveArtifact(artifact, artifactFilename("thread", artifact.thread_id))}
          >
            {t("admin.threadScraping.downloadThread")}
          </Button>
        )}
      </div>

      {error && <p className="mt-4 text-sm text-red-200" role="alert">{error}</p>}
      {isLoading && <p className="mt-4 text-sm text-iron-300">{t("common.loading")}</p>}
      {!isLoading && threads.length === 0 && (
        <p className="mt-4 text-sm text-iron-300">{t("admin.threadScraping.empty")}</p>
      )}

      {threads.length > 0 && (
        <div className="mt-4 grid gap-4 lg:grid-cols-[minmax(0,18rem)_minmax(0,1fr)]">
          <div className="max-h-[32rem] space-y-2 overflow-y-auto pr-1">
            {threads.map((thread) => (
              <button
                key={thread.thread_id}
                type="button"
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
                  {(artifact.messages || []).map((message) => (
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
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </Panel>
  );
}
