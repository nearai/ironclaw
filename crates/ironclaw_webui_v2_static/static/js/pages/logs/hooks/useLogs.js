import { React } from "../../../lib/html.js";

// TODO: requires v2 log-streaming endpoint. The v2 ingress only
// exposes per-thread event streams; system-wide log tail is not in
// the #3815 contract. Hook returns empty/static so the Logs page
// renders an empty list without hitting any v1 path.
export function useLogs() {
  const [level, setLevel] = React.useState("info");

  const updateLevel = React.useCallback(async (next) => {
    setLevel(next); // local-only — no v2 endpoint to persist
  }, []);

  return {
    entries: [],
    level,
    setLevel: updateLevel,
    status: "todo",
    isLoading: false,
  };
}
