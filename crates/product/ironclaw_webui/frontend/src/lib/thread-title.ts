function threadIdFor(record) {
  return record?.thread_id || record?.id || null;
}

export function normalizeSidebarTitle(title, threadId) {
  const trimmed = String(title || "").trim();
  if (!trimmed) return null;
  if (threadId && trimmed === String(threadId)) return null;
  return trimmed;
}

export function displaySidebarTitle(thread, fallback = "Untitled thread") {
  return normalizeSidebarTitle(thread?.title, threadIdFor(thread)) || fallback;
}
