import { queryClient } from "../../../lib/query-client.js";

const THREADS_QUERY_KEY = ["threads"];
const TITLE_MAX_CHARS = 60;

export function deriveSidebarTitle(message) {
  const firstLine = String(message || "")
    .split(/\r?\n/)
    .find((line) => line.trim());
  if (!firstLine) return null;
  const trimmed = firstLine.trim();
  const chars = Array.from(trimmed);
  if (chars.length <= TITLE_MAX_CHARS) return trimmed;
  return `${chars.slice(0, TITLE_MAX_CHARS - 3).join("")}...`;
}

function threadIdFor(record) {
  return record?.thread_id || record?.id || null;
}

function threadListData(data) {
  return data && typeof data === "object"
    ? data
    : { threads: [], next_cursor: null };
}

function withNormalizedThreadId(record) {
  const threadId = threadIdFor(record);
  if (!threadId) return null;
  return { ...record, thread_id: threadId };
}

export function upsertThreadList(data, thread) {
  const record = withNormalizedThreadId(thread);
  if (!record) return data;

  const current = threadListData(data);
  const threads = Array.isArray(current.threads) ? current.threads : [];
  let promoted = null;
  const remaining = [];

  for (const existing of threads) {
    if (threadIdFor(existing) !== record.thread_id) {
      remaining.push(existing);
      continue;
    }
    if (!promoted) {
      promoted = { ...existing, ...record, thread_id: record.thread_id };
    }
  }

  return {
    ...current,
    threads: [promoted || record, ...remaining],
    next_cursor: current.next_cursor ?? null,
  };
}

export function touchThreadList(data, { threadId, messageContent, updatedAt }) {
  if (!threadId) return data;

  const current = threadListData(data);
  const threads = Array.isArray(current.threads) ? current.threads : [];
  const derivedTitle = deriveSidebarTitle(messageContent);
  const timestamp = updatedAt || new Date().toISOString();
  let promoted = null;
  const remaining = [];

  for (const existing of threads) {
    if (threadIdFor(existing) !== threadId) {
      remaining.push(existing);
      continue;
    }
    if (!promoted) {
      promoted = {
        ...existing,
        thread_id: threadId,
        title: existing.title || derivedTitle || null,
        updated_at: timestamp,
      };
    }
  }

  if (!promoted) {
    promoted = {
      thread_id: threadId,
      title: derivedTitle,
      created_at: timestamp,
      updated_at: timestamp,
    };
  }

  return {
    ...current,
    threads: [promoted, ...remaining],
    next_cursor: current.next_cursor ?? null,
  };
}

export function upsertThreadInCache(thread) {
  queryClient.setQueryData?.(THREADS_QUERY_KEY, (data) =>
    upsertThreadList(data, thread),
  );
}

export function touchThreadInCache(update) {
  queryClient.setQueryData?.(THREADS_QUERY_KEY, (data) =>
    touchThreadList(data, update),
  );
}
