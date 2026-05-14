import { React } from "../../../lib/html.js";
import { apiFetch, readStoredToken } from "../../../lib/api.js";

const LOG_MAX_ENTRIES = 2000;

export function useLogs() {
  const [entries, setEntries] = React.useState([]);
  const [paused, setPaused] = React.useState(false);
  const [levelFilter, setLevelFilter] = React.useState("all");
  const [targetFilter, setTargetFilter] = React.useState("");
  const [autoScroll, setAutoScroll] = React.useState(true);
  const [serverLevel, setServerLevel] = React.useState(null);
  const bufferRef = React.useRef([]);
  const pausedRef = React.useRef(false);

  pausedRef.current = paused;

  React.useEffect(() => {
    const token = readStoredToken();
    const url = token
      ? `/api/logs/events?token=${encodeURIComponent(token)}`
      : "/api/logs/events";

    const es = new EventSource(url);

    es.addEventListener("log", (e) => {
      let entry;
      try {
        entry = JSON.parse(e.data);
      } catch {
        return;
      }
      if (pausedRef.current) {
        bufferRef.current.push(entry);
        return;
      }
      setEntries((prev) => {
        const next = [entry, ...prev];
        return next.length > LOG_MAX_ENTRIES ? next.slice(0, LOG_MAX_ENTRIES) : next;
      });
    });

    return () => es.close();
  }, []);

  React.useEffect(() => {
    apiFetch("/api/logs/level")
      .then((data) => setServerLevel(data.level))
      .catch(() => {});
  }, []);

  const resume = React.useCallback(() => {
    setPaused(false);
    const buffered = bufferRef.current;
    bufferRef.current = [];
    if (buffered.length > 0) {
      setEntries((prev) => {
        const next = [...buffered.reverse(), ...prev];
        return next.length > LOG_MAX_ENTRIES ? next.slice(0, LOG_MAX_ENTRIES) : next;
      });
    }
  }, []);

  const togglePause = React.useCallback(() => {
    if (paused) {
      resume();
    } else {
      setPaused(true);
    }
  }, [paused, resume]);

  const clearEntries = React.useCallback(() => {
    setEntries([]);
    bufferRef.current = [];
  }, []);

  const changeServerLevel = React.useCallback((level) => {
    apiFetch("/api/logs/level", { method: "PUT", body: JSON.stringify({ level }) })
      .then((data) => setServerLevel(data.level))
      .catch(() => {});
  }, []);

  const visibleEntries = React.useMemo(() => {
    return entries.filter((e) => {
      const matchesLevel = levelFilter === "all" || e.level === levelFilter;
      const matchesTarget =
        !targetFilter || e.target.toLowerCase().includes(targetFilter.toLowerCase());
      return matchesLevel && matchesTarget;
    });
  }, [entries, levelFilter, targetFilter]);

  return {
    entries: visibleEntries,
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
    serverLevel,
    changeServerLevel,
  };
}
