import hotToast, { type ToastOptions as HotToastOptions } from "react-hot-toast";

/* Keep the app-facing API small so callers do not depend directly on the
   rendering library or repeat the product's duration/accessibility defaults. */
export type ToastTone = "info" | "success" | "error";

type ToastOptions = {
  tone?: ToastTone;
  duration?: number;
};

export const DEFAULT_TOAST_DURATION = 2600;
export const DEFAULT_ERROR_TOAST_DURATION = 8000;

export function toast(message: string, opts: ToastOptions = {}) {
  const tone = opts.tone || "info";
  const options: HotToastOptions = {
    duration:
      opts.duration ??
      (tone === "error" ? DEFAULT_ERROR_TOAST_DURATION : DEFAULT_TOAST_DURATION),
    ariaProps:
      tone === "error"
        ? { role: "alert", "aria-live": "assertive" }
        : { role: "status", "aria-live": "polite" },
  };

  if (tone === "error") return hotToast.error(message, options);
  if (tone === "success") return hotToast.success(message, options);
  return hotToast(message, options);
}
