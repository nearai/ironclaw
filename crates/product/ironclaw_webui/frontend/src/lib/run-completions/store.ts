// In-page cache of unread run-completion notices, for the badge and the
// notification list. The durable projection stays authoritative: this cache
// rebases from the unread snapshot on boot and on stream rebase, applies
// notice/clear events in sequence order, and never invents read state
// (§9.3: clearing follows the server's clear event or a settled mutation
// response, not local dismissal).

import type { RunCompletionNotice } from "./protocol";

// Inlined rather than imported so the eager /chat closure carries only this
// module (the hook needs the badge cache at boot); the full wire protocol
// stays in the lazily imported orchestrator graph.
function compareSequences(a: string, b: string): number {
  if (a.length !== b.length) return a.length - b.length;
  return a < b ? -1 : a > b ? 1 : 0;
}

// §5.4: the UI cache retains at most 250 active notices; eviction only
// affects cache acceleration because the durable projection remains
// authoritative.
const MAX_CACHED_NOTICES = 250;

export type RunCompletionStoreSnapshot = {
  /** Unread notices, newest first. */
  notices: RunCompletionNotice[];
  unreadCount: number;
  /** Greatest sequence applied; the stream resume position. */
  resumeSequence: string;
};

type Listener = () => void;

let noticesById = new Map<string, RunCompletionNotice>();
let resumeSequence = "0";
let snapshot: RunCompletionStoreSnapshot = {
  notices: [],
  unreadCount: 0,
  resumeSequence,
};
const listeners = new Set<Listener>();

function rebuildSnapshot() {
  const notices = Array.from(noticesById.values())
    .filter((notice) => !notice.read)
    .sort((a, b) => compareSequences(b.sequence, a.sequence));
  while (notices.length > MAX_CACHED_NOTICES) {
    const evicted = notices.pop();
    if (evicted) noticesById.delete(evicted.notice_id);
  }
  snapshot = {
    notices,
    unreadCount: notices.length,
    resumeSequence,
  };
  for (const listener of listeners) listener();
}

function advanceSequence(sequence: string) {
  if (compareSequences(sequence, resumeSequence) > 0) {
    resumeSequence = sequence;
  }
}

export function subscribeRunCompletionStore(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function runCompletionSnapshot(): RunCompletionStoreSnapshot {
  return snapshot;
}

/** Replace the cache with the durable unread snapshot (boot / rebase). */
export function rebaseFromSnapshot(
  notices: RunCompletionNotice[],
  snapshotResumeSequence: string,
) {
  noticesById = new Map(
    notices
      .filter((notice) => !notice.read)
      .map((notice) => [notice.notice_id, notice]),
  );
  resumeSequence = "0";
  advanceSequence(snapshotResumeSequence);
  for (const notice of notices) advanceSequence(notice.sequence);
  rebuildSnapshot();
}

export function applyNotice(notice: RunCompletionNotice) {
  advanceSequence(notice.sequence);
  if (notice.read) {
    noticesById.delete(notice.notice_id);
  } else {
    noticesById.set(notice.notice_id, notice);
  }
  rebuildSnapshot();
}

export function applyClear(noticeId: string, sequence?: string) {
  if (sequence) advanceSequence(sequence);
  if (noticesById.delete(noticeId)) {
    rebuildSnapshot();
  } else if (sequence) {
    rebuildSnapshot();
  }
}

/** Settle a set of notice ids (mutation responses echo settled ids). */
export function applySettled(noticeIds: string[]) {
  let changed = false;
  for (const noticeId of noticeIds) {
    changed = noticesById.delete(noticeId) || changed;
  }
  if (changed) rebuildSnapshot();
}

export function noticeForRun(runId: string): RunCompletionNotice | null {
  for (const notice of noticesById.values()) {
    if (notice.run_id === runId) return notice;
  }
  return null;
}

export function unreadForThread(threadId: string): RunCompletionNotice[] {
  return snapshot.notices.filter((notice) => notice.thread_id === threadId);
}

/** Greatest unread sequence for one thread (thread_read evidence input). */
export function maxSequenceForThread(threadId: string): string | null {
  let max: string | null = null;
  for (const notice of noticesById.values()) {
    if (notice.thread_id !== threadId) continue;
    if (max === null || compareSequences(notice.sequence, max) > 0) {
      max = notice.sequence;
    }
  }
  return max;
}

/** Clear all cached notices (sign-out / orchestrator stop, and tests). */
export function resetRunCompletionStore() {
  noticesById = new Map();
  resumeSequence = "0";
  rebuildSnapshot();
}

export const resetRunCompletionStoreForTests = resetRunCompletionStore;
