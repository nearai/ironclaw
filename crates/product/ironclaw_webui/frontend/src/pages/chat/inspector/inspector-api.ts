import { apiFetch, readStoredToken } from "../../../lib/api";

const INSPECTOR_BASE = "/api/webchat/v2/operator/inspector";

function runPath(threadId: string, runId: string): string {
  return `${INSPECTOR_BASE}/threads/${encodeURIComponent(threadId)}/runs/${encodeURIComponent(runId)}`;
}

export function fetchInspectorSnapshot({
  threadId,
  runId,
  signal,
}: {
  threadId: string;
  runId: string;
  signal?: AbortSignal;
}): Promise<unknown> {
  return apiFetch(runPath(threadId, runId), { signal });
}

export function inspectorEventStreamRequest({
  threadId,
  runId,
}: {
  threadId: string;
  runId: string;
}): { url: string; headers: () => Record<string, string> } {
  const url = new URL(`${runPath(threadId, runId)}/events`, window.location.origin);
  return {
    url: url.toString(),
    headers: () => {
      const token = readStoredToken();
      return token ? { Authorization: `Bearer ${token}` } : {};
    },
  };
}
