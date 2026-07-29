// DEMO-mode bridge for the page API modules whose endpoints are TODO stubs.
//
// Most of the SPA reaches the network through `apiFetch`, so the DEMO fetch
// interceptor covers it automatically. A few page API modules (jobs,
// routines, missions) instead resolve hardcoded empty payloads because their
// v2 contracts have not landed — they never issue a request, so there is
// nothing to intercept.
//
// `demoJson` lets those modules opt into the fixture router without changing
// production behavior: `DEMO_MODE` is a build-time constant, so a normal
// build keeps the `fallback` path and drops the fetch branch entirely.

export const DEMO_MODE = import.meta.env.VITE_DEMO_MODE === "1";

export async function demoJson<T>(path: string, fallback: T): Promise<T> {
  if (!DEMO_MODE) return fallback;
  try {
    const response = await fetch(path, { headers: { Accept: "application/json" } });
    if (!response.ok) return fallback;
    return (await response.json()) as T;
  } catch {
    // Keep the walkthrough resilient: a missing fixture degrades to the
    // same empty state production shows.
    return fallback;
  }
}
