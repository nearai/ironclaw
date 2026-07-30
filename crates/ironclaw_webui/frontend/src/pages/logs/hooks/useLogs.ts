// @ts-nocheck
import { useLocation } from "react-router";
import React from "react";
import { queryLogs, queryOperatorLogs } from "../../../lib/api";
import { normalizeOperatorLogsResponse } from "../lib/logs-data";

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

function mergeLogEntries(...pages) {
  const seen = new Set();
  const merged = [];
  for (const page of pages) {
    for (const entry of page) {
      if (seen.has(entry.id)) continue;
      seen.add(entry.id);
      merged.push(entry);
    }
  }
  return merged;
}

function effectiveLocationSearch(location = globalThis.location) {
  return location?.search || globalThis.location?.search || "";
}

export function readLogScopeFromLocation(location = globalThis.location, defaultThreadId = null) {
  const params = new URLSearchParams(effectiveLocationSearch(location));
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
  const locationSearch = effectiveLocationSearch(location);
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
  const [nextCursor, setNextCursor] = React.useState(null);
  const [isLoadingMore, setIsLoadingMore] = React.useState(false);
  const [loadMoreError, setLoadMoreError] = React.useState(null);
  const hiddenEntryIdsRef = React.useRef(new Set());
  const generationRef = React.useRef(0);
  const refreshRequestIdRef = React.useRef(0);
  const nextCursorRef = React.useRef(null);
  const olderEntriesRef = React.useRef([]);
  const hasLoadedOlderRef = React.useRef(false);
  const loadMoreInFlightRef = React.useRef(null);
  const needsThreadScope = !isAdmin && !threadId;

  React.useEffect(() => {
    generationRef.current += 1;
    refreshRequestIdRef.current += 1;
    nextCursorRef.current = null;
    olderEntriesRef.current = [];
    hasLoadedOlderRef.current = false;
    loadMoreInFlightRef.current = null;
    setEntries([]);
    setError(null);
    setNextCursor(null);
    setIsLoadingMore(false);
    setLoadMoreError(null);
  }, [
    isAdmin,
    levelFilter,
    runId,
    source,
    targetFilter,
    threadId,
    toolCallId,
    toolName,
    turnId,
  ]);

  const queryLogPage = React.useCallback(async (request) => {
    try {
      return await (isAdmin ? queryOperatorLogs(request) : queryLogs(request));
    } catch (err) {
      if (!isAdmin || !TERMINAL_UNSUPPORTED_STATUSES.has(err?.status)) {
        throw err;
      }
      return queryLogs(request);
    }
  }, [isAdmin]);

  const requestForCursor = React.useCallback((cursor = null) => ({
    limit: LOG_LIMIT,
    cursor,
    level: levelFilter === "all" ? null : levelFilter,
    target: targetFilter.trim() || null,
    threadId,
    runId,
    turnId,
    toolCallId,
    toolName,
    source,
  }), [
    levelFilter,
    runId,
    source,
    targetFilter,
    threadId,
    toolCallId,
    toolName,
    turnId,
  ]);

  const loadLogs = React.useCallback(async () => {
    if (needsThreadScope) {
      setIsLoading(false);
      return;
    }
    const generation = generationRef.current;
    const requestId = ++refreshRequestIdRef.current;
    setIsLoading(true);
    try {
      const response = await queryLogPage(requestForCursor());
      if (
        generation !== generationRef.current ||
        requestId !== refreshRequestIdRef.current
      ) return;
      const hidden = hiddenEntryIdsRef.current;
      const logs = normalizeOperatorLogsResponse(response);
      const nextEntries = logs.entries.filter((entry) => !hidden.has(entry.id));
      setEntries(mergeLogEntries(nextEntries, olderEntriesRef.current));
      if (!hasLoadedOlderRef.current) {
        nextCursorRef.current = logs.nextCursor;
        setNextCursor(logs.nextCursor);
      }
      setError(null);
    } catch (err) {
      if (
        generation !== generationRef.current ||
        requestId !== refreshRequestIdRef.current
      ) return;
      setError(err);
    } finally {
      if (
        generation === generationRef.current &&
        requestId === refreshRequestIdRef.current
      ) {
        setIsLoading(false);
      }
    }
  }, [
    needsThreadScope,
    queryLogPage,
    requestForCursor,
  ]);

  const loadOlder = React.useCallback(() => {
    const cursor = nextCursorRef.current;
    if (!cursor) return Promise.resolve();
    if (loadMoreInFlightRef.current) return loadMoreInFlightRef.current;

    const generation = generationRef.current;
    setIsLoadingMore(true);
    setLoadMoreError(null);
    const request = queryLogPage(requestForCursor(cursor))
      .then((response) => {
        if (generation !== generationRef.current) return;
        const hidden = hiddenEntryIdsRef.current;
        const logs = normalizeOperatorLogsResponse(response);
        const pageEntries = logs.entries.filter((entry) => !hidden.has(entry.id));
        olderEntriesRef.current = mergeLogEntries(
          olderEntriesRef.current,
          pageEntries,
        );
        hasLoadedOlderRef.current = true;
        nextCursorRef.current = logs.nextCursor;
        setEntries((current) => mergeLogEntries(current, pageEntries));
        setNextCursor(logs.nextCursor);
        setLoadMoreError(null);
      })
      .catch((err) => {
        if (generation === generationRef.current) {
          setLoadMoreError(err);
        }
      })
      .finally(() => {
        if (loadMoreInFlightRef.current === request) {
          loadMoreInFlightRef.current = null;
          if (generation === generationRef.current) {
            setIsLoadingMore(false);
          }
        }
      });
    loadMoreInFlightRef.current = request;
    return request;
  }, [queryLogPage, requestForCursor]);

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
    generationRef.current += 1;
    refreshRequestIdRef.current += 1;
    nextCursorRef.current = null;
    olderEntriesRef.current = [];
    hasLoadedOlderRef.current = false;
    loadMoreInFlightRef.current = null;
    setEntries([]);
    setNextCursor(null);
    setIsLoadingMore(false);
    setLoadMoreError(null);
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
    nextCursor,
    isLoadingMore,
    loadMoreError,
    loadOlder,
  };
}
