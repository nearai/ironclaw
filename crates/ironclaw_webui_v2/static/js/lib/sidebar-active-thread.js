export function activeSidebarThreadIdFromPath(pathname) {
  if (typeof pathname !== "string") return null;

  const trimmed = pathname.replace(/\/+$/, "");
  if (!trimmed.startsWith("/chat/")) return null;

  const remainder = trimmed.slice("/chat/".length);
  if (!remainder || remainder.includes("/")) return null;

  try {
    return decodeURIComponent(remainder);
  } catch {
    return remainder;
  }
}
