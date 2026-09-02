// Pure read-evidence helpers for the run-completion client (2026-08-13
// design §7.6, §9.3). Kept free of transport and DOM dependencies so the
// orchestrator's evidence rules are unit-testable on their own.

import { hasObservedRun } from "./coordination";
import { compareSequences } from "./sequence";
import { unreadForThread } from "./store";

/**
 * The resume token for the owner stream after completion `sequence`. The
 * server admits only the JSON-quoted cursor token it hands out itself (the
 * same form every event frame carries), so the `rc:` namespace is quoted
 * here; a raw `rc:N` would be rejected as a cursor the server never issued.
 */
export function resumeCursorFor(sequence: string): string {
  return JSON.stringify(`rc:${sequence}`);
}

/**
 * The greatest completion sequence for `threadId` whose reply this profile
 * has actually rendered — from the loaded history or a live finalization.
 * Notices newer than that stay unread until their reply is on screen:
 * display is not read (§9.3), and a badge change alone never settles.
 */
export function observedThroughSequence(threadId: string): string | null {
  let best: string | null = null;
  for (const notice of unreadForThread(threadId)) {
    if (!hasObservedRun(notice.run_id)) continue;
    if (best === null || compareSequences(notice.sequence, best) > 0) {
      best = notice.sequence;
    }
  }
  return best;
}
