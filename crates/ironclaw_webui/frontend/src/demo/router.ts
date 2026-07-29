// DEMO-mode fetch router: matches (method, pathname) against the registered
// fixture routes and builds a `Response`. Unknown API paths resolve to an
// empty JSON object so an unmocked corner of the product degrades to an
// empty state instead of an error banner.

import type { DemoRequest, DemoResponse, DemoRoute } from "./types";

const routes: DemoRoute[] = [];

export function registerDemoRoutes(newRoutes: DemoRoute[]) {
  routes.push(...newRoutes);
}

function toResponse(demo: DemoResponse): Response {
  const status = demo.status ?? 200;
  if (demo.text !== undefined) {
    return new Response(demo.text, {
      status,
      headers: { "Content-Type": demo.contentType || "text/plain; charset=utf-8" },
    });
  }
  return new Response(JSON.stringify(demo.json ?? {}), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

/** Paths the demo router owns; anything else falls through to real fetch. */
export function isDemoPath(path: string): boolean {
  return path.startsWith("/api/") || path.startsWith("/auth/");
}

export async function handleDemoRequest(
  input: RequestInfo | URL,
  init?: RequestInit
): Promise<Response | null> {
  const rawUrl =
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input.href
        : input.url;
  const url = new URL(rawUrl, window.location.origin);
  if (url.origin !== window.location.origin || !isDemoPath(url.pathname)) {
    return null;
  }

  const method = (
    init?.method ||
    (typeof input === "object" && "method" in input ? input.method : "") ||
    "GET"
  ).toUpperCase();

  let body: Record<string, unknown> | null = null;
  const rawBody = init?.body;
  if (typeof rawBody === "string") {
    try {
      body = JSON.parse(rawBody);
    } catch {
      body = null;
    }
  }

  const request: DemoRequest = { method, path: url.pathname, url, body };

  for (const route of routes) {
    if (route.method !== method) continue;
    const match = route.pattern.exec(url.pathname);
    if (!match) continue;
    const demo = route.handle(request, match);
    if (demo) return toResponse(demo);
  }

  // Unmocked endpoint: succeed with an empty object so consumers render
  // their empty states. Logged so a missing fixture is easy to spot.
  console.debug(`[demo] unmocked ${method} ${url.pathname} -> {}`);
  return toResponse({ json: {} });
}
