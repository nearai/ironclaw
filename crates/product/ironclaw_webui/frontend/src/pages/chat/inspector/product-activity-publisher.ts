import type { PublishProductInspectorActivity } from "./product-activity";

export function publishProductInspectorActivity(
  input: PublishProductInspectorActivity,
): void {
  if (new URLSearchParams(window.location.search).get("debug") !== "true") return;
  void import("./product-activity").then(({ publishProductInspectorActivity: publish }) => {
    publish(input);
  });
}
