export function activeRouteThreadIdFromPath(pathname) {
  if (typeof pathname !== "string") return null;

  const cleanPath = pathname.split(/[?#]/)[0];
  const trimmed = cleanPath.replace(/\/+$/, "");
  if (!trimmed.startsWith("/chat/")) return null;

  const remainder = trimmed.slice("/chat/".length);
  if (!remainder || remainder.includes("/")) return null;

  try {
    return decodeURIComponent(remainder);
  } catch {
    return remainder;
  }
}

export function routeSynchronizedThreadsState(threadsState, pathname) {
  return {
    ...threadsState,
    activeThreadId: activeRouteThreadIdFromPath(pathname),
  };
}
