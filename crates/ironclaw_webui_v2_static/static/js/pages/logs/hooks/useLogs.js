import { React } from "../../../lib/html.js";
import { queryOperatorLogs } from "../../../lib/api.js";
import { normalizeOperatorLogsResponse } from "../lib/logs-data.js";

const POLL_INTERVAL_MS = 2000;
const LOG_LIMIT = 500;
const HIDDEN_ENTRY_ID_CAP = 2000;
const TERMINAL_UNSUPPORTED_STATUSES = new Set([403, 404]);

export function useLogs() {
  const [entries, setEntries] = React.useState([]);
  const [levelFilter, setLevelFilter] = React.useState("all");
  const [targetFilter, setTargetFilter] = React.useState("");
  const [paused, setPaused] = React.useState(false);
  const [autoScroll, setAutoScroll] = React.useState(true);
  const [isLoading, setIsLoading] = React.useState(true);
  const [error, setError] = React.useState(null);
  const [isUnsupported, setIsUnsupported] = React.useState(false);
  const hiddenEntryIdsRef = React.useRef(new Set());
  const requestIdRef = React.useRef(0);

  const loadLogs = React.useCallback(async () => {
    if (isUnsupported) return;
    const requestId = ++requestIdRef.current;
    setIsLoading(true);
    try {
      const response = await queryOperatorLogs({
        limit: LOG_LIMIT,
        level: levelFilter === "all" ? null : levelFilter,
        target: targetFilter.trim() || null,
      });
      if (requestId !== requestIdRef.current) return;
      const hidden = hiddenEntryIdsRef.current;
      const logs = normalizeOperatorLogsResponse(response);
      const nextEntries = logs.entries.filter((entry) => !hidden.has(entry.id));
      setEntries(nextEntries);
      setError(null);
    } catch (err) {
      if (requestId !== requestIdRef.current) return;
      if (TERMINAL_UNSUPPORTED_STATUSES.has(err?.status)) {
        setEntries([]);
        setError(null);
        setIsUnsupported(true);
        return;
      }
      setError(err);
    } finally {
      if (requestId === requestIdRef.current) {
        setIsLoading(false);
      }
    }
  }, [isUnsupported, levelFilter, targetFilter]);

  React.useEffect(() => {
    if (paused || isUnsupported) return undefined;
    loadLogs();
    const timer = setInterval(loadLogs, POLL_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [isUnsupported, loadLogs, paused]);

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
    status: error ? "error" : isLoading ? "loading" : "ready",
    isLoading,
    error,
  };
}
