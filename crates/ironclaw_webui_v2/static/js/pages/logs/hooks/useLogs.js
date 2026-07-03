import { useLocation } from "react-router";
import { React } from "../../../lib/html.js";
import { queryLogs, queryOperatorLogs } from "../../../lib/api.js";
import { normalizeOperatorLogsResponse } from "../lib/logs-data.js";

const POLL_INTERVAL_MS = 2000;
const LOG_LIMIT = 500;
const HIDDEN_ENTRY_ID_CAP = 2000;
const TERMINAL_UNSUPPORTED_STATUSES = new Set([403, 404]);
const SCOPE_QUERY_PARAMS = [
  ["threadId", "thread_id", "logs.scope.thread"],
  ["runId", "run_id", "logs.scope.run"],
  ["turnId", "turn_id", "logs.scope.turn"],
  ["toolCallId", "tool_call_id", "logs.scope.toolCall"],
  ["toolName", "tool_name", "logs.scope.tool"],
  ["source", "source", "logs.scope.source"],
];

export function readLogScopeFromLocation(location = globalThis.location, defaultThreadId = null) {
  const params = new URLSearchParams(location?.search || "");
  const scope = { active: [] };
  for (const [key, param, labelKey] of SCOPE_QUERY_PARAMS) {
    const value = params.get(param)?.trim();
    if (value) {
      scope[key] = value;
      scope.active.push({ key, param, labelKey, value });
    } else {
      scope[key] = null;
    }
  }
  if (!scope.threadId && defaultThreadId) {
    scope.threadId = defaultThreadId;
  }
  return scope;
}

// Fail closed to caller-scoped logs if layout context is missing. Operator logs
// are an optimization for operator-capable sessions, not the default.
export function useLogs({ isAdmin = false, defaultThreadId = null } = {}) {
  const location = useLocation();
  const locationSearch = location?.search || "";
  const scope = React.useMemo(
    () => readLogScopeFromLocation(location, defaultThreadId),
    [defaultThreadId, locationSearch]
  );
  const { runId, source, threadId, toolCallId, toolName, turnId } = scope;
  const [entries, setEntries] = React.useState([]);
  const [levelFilter, setLevelFilter] = React.useState("all");
  const [targetFilter, setTargetFilter] = React.useState("");
  const [paused, setPaused] = React.useState(false);
  const [autoScroll, setAutoScroll] = React.useState(true);
  const [isLoading, setIsLoading] = React.useState(true);
  const [error, setError] = React.useState(null);
  const hiddenEntryIdsRef = React.useRef(new Set());
  const requestIdRef = React.useRef(0);
  const needsThreadScope = !isAdmin && !threadId;

  React.useEffect(() => {
    requestIdRef.current += 1;
    setEntries([]);
    setError(null);
  }, [isAdmin, runId, source, threadId, toolCallId, toolName, turnId]);

  const loadLogs = React.useCallback(async () => {
    if (needsThreadScope) {
      setIsLoading(false);
      return;
    }
    const requestId = ++requestIdRef.current;
    setIsLoading(true);
    try {
      const request = {
        limit: LOG_LIMIT,
        level: levelFilter === "all" ? null : levelFilter,
        target: targetFilter.trim() || null,
        threadId,
        runId,
        turnId,
        toolCallId,
        toolName,
        source,
      };
      let response;
      try {
        response = await (isAdmin ? queryOperatorLogs(request) : queryLogs(request));
      } catch (err) {
        if (!isAdmin || !TERMINAL_UNSUPPORTED_STATUSES.has(err?.status)) {
          throw err;
        }
        response = await queryLogs(request);
      }
      if (requestId !== requestIdRef.current) return;
      const hidden = hiddenEntryIdsRef.current;
      const logs = normalizeOperatorLogsResponse(response);
      const nextEntries = logs.entries.filter((entry) => !hidden.has(entry.id));
      setEntries(nextEntries);
      setError(null);
    } catch (err) {
      if (requestId !== requestIdRef.current) return;
      setError(err);
    } finally {
      if (requestId === requestIdRef.current) {
        setIsLoading(false);
      }
    }
  }, [
    isAdmin,
    levelFilter,
    needsThreadScope,
    runId,
    source,
    targetFilter,
    threadId,
    toolCallId,
    toolName,
    turnId,
  ]);

  React.useEffect(() => {
    loadLogs();
  }, [loadLogs]);

  React.useEffect(() => {
    if (paused || needsThreadScope) return undefined;
    const timer = setInterval(loadLogs, POLL_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [loadLogs, needsThreadScope, paused]);

  const togglePause = React.useCallback(() => {
    setPaused((value) => !value);
  }, []);

  const clearEntries = React.useCallback(() => {
    const hidden = [
      ...hiddenEntryIdsRef.current,
      ...entries.map((entry) => entry.id),
    ].slice(-HIDDEN_ENTRY_ID_CAP);
    hiddenEntryIdsRef.current = new Set(hidden);
    setEntries([]);
  }, [entries]);

  return {
    entries,
    totalCount: entries.length,
    paused,
    togglePause,
    clearEntries,
    levelFilter,
    setLevelFilter,
    targetFilter,
    setTargetFilter,
    autoScroll,
    setAutoScroll,
    serverLevel: null,
    changeServerLevel: async () => {},
    scope,
    needsThreadScope,
    status: needsThreadScope ? "needs_scope" : error ? "error" : isLoading ? "loading" : "ready",
    isLoading,
    error,
  };
}
