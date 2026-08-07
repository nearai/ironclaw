// @ts-nocheck

/* Per-thread run bookkeeping for the chat event handler.
 *
 * These three slots are scoped to ONE thread. They used to be private
 * `React.useRef`s inside `useChatEvents`, which owned them but not their
 * lifecycle — the thread-switch reset lives in `useChat`, so nothing ever
 * cleared them. `latestRunId` is only cleared on a TERMINAL run status, so a
 * run that got stuck pinned it for the life of the mounted chat page and it
 * followed the user into every thread they opened afterwards, where it was
 * consumed as the run-id fallback for untagged capability frames and as the
 * seed for the stale-terminal check (silently dropping the new thread's own
 * terminal status, so its timeline was never refetched).
 *
 * Bundling them behind one owner keeps `useChatEvents` stateless: `useChat`
 * holds this ref alongside the other per-thread state it already resets, so
 * "what does a thread switch mean" is answered in exactly one place. The
 * slots stay ref-shaped (`{ current }`) so the handler's helpers can keep
 * mutating them directly.
 */
export function createRunTrackingState() {
  return {
    // Run ids already handed to `onRunSettled`, so SSE replays (reconnect
    // with last-event-id, repeated snapshots) settle each run exactly once.
    settledRuns: { current: new Set() },
    // Most recent run id observed on this thread. Used to reject stale
    // terminal statuses and to scope capability frames that omit a run id.
    latestRunId: { current: null },
    // Run id whose gate prompt is currently displayed.
    promptRunId: { current: null },
  };
}

export function resetRunTrackingState(stateRef) {
  if (!stateRef) return;
  stateRef.current = createRunTrackingState();
}
