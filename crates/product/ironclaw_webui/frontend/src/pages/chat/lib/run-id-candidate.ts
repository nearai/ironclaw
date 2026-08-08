export const AMBIGUOUS_RUN_ID: unique symbol = Symbol("ambiguous-run-id");

export type RunIdCandidate = string | null | typeof AMBIGUOUS_RUN_ID;

export function mergeRunIdCandidate(
  current: RunIdCandidate,
  runId: unknown,
): RunIdCandidate {
  if (typeof runId !== "string" || runId.length === 0) return current;
  if (current === null) return runId;
  return current === runId ? current : AMBIGUOUS_RUN_ID;
}
