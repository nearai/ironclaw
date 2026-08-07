import { inspectorDebugEnabled } from "./inspector-shell";

export function publishProductInspectorEnvelope(
  envelope: unknown,
  threadId: unknown,
  fallbackRunId: unknown,
): void {
  if (
    typeof window === "undefined"
    || !inspectorDebugEnabled(window.location.search)
  ) return;
  void import("./product-activity-envelope").then(({ publishProductInspectorEnvelope: publish }) => {
    publish(envelope, threadId, fallbackRunId);
  });
}
