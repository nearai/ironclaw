const RAW_THREAD_TITLE_RE = /^thread[_-][A-Za-z0-9_-]{6,}$/;

function threadIdFor(record) {
  return record?.thread_id || record?.id || null;
}

export function normalizeSidebarTitle(title, threadId) {
  const trimmed = String(title || "").trim();
  if (!trimmed) return null;
  if (threadId && trimmed === threadId) return null;
  if (RAW_THREAD_TITLE_RE.test(trimmed)) return null;
  return trimmed;
}

export function displaySidebarTitle(thread, fallback = "Untitled thread") {
  return normalizeSidebarTitle(thread?.title, threadIdFor(thread)) || fallback;
}
