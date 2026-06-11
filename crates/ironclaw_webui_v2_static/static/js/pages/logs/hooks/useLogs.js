import { React } from "../../../lib/html.js";
import { queryOperatorLogs } from "../../../lib/api.js";

const POLL_INTERVAL_MS = 2000;
const LOG_LIMIT = 500;

function normalizeEntry(entry) {
  return {
    id: String(entry.id ?? `${entry.timestamp}:${entry.target}:${entry.message}`),
    timestamp: entry.timestamp || "",
    level: String(entry.level || "info").toLowerCase(),
    target: entry.target || "",
    message: entry.message || "",
  };
}

export function useLogs() {
  const [entries, setEntries] = React.useState([]);
  const [levelFilter, setLevelFilter] = React.useState("all");
  const [targetFilter, setTargetFilter] = React.useState("");
  const [paused, setPaused] = React.useState(false);
  const [autoScroll, setAutoScroll] = React.useState(true);
  const [isLoading, setIsLoading] = React.useState(true);
  const [error, setError] = React.useState(null);
  const hiddenEntryIdsRef = React.useRef(new Set());
  const requestIdRef = React.useRef(0);

  const loadLogs = React.useCallback(async () => {
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
      const nextEntries = (response.entries || [])
        .map(normalizeEntry)
        .filter((entry) => !hidden.has(entry.id));
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
  }, [levelFilter, targetFilter]);

  React.useEffect(() => {
    if (paused) return undefined;
    loadLogs();
    const timer = setInterval(loadLogs, POLL_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [loadLogs, paused]);

  const togglePause = React.useCallback(() => {
    setPaused((value) => !value);
  }, []);

  const clearEntries = React.useCallback(() => {
    hiddenEntryIdsRef.current = new Set([
      ...hiddenEntryIdsRef.current,
      ...entries.map((entry) => entry.id),
    ]);
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
