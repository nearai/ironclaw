import assert from "node:assert/strict";
import { beforeEach, test, vi } from "vitest";

const hotToast = vi.hoisted(() => ({
  blank: vi.fn((..._args: unknown[]) => "blank-id"),
  error: vi.fn((..._args: unknown[]) => "error-id"),
  success: vi.fn((..._args: unknown[]) => "success-id"),
}));

vi.mock("react-hot-toast", () => ({
  default: Object.assign(hotToast.blank, {
    error: hotToast.error,
    success: hotToast.success,
  }),
}));

import {
  DEFAULT_ERROR_TOAST_DURATION,
  DEFAULT_TOAST_DURATION,
  toast,
} from "./toast";

beforeEach(() => {
  vi.clearAllMocks();
});

test("toast delegates tones with product duration and accessibility defaults", () => {
  assert.equal(toast("Info"), "blank-id");
  assert.equal(toast("Saved", { tone: "success" }), "success-id");
  assert.equal(toast("Could not save", { tone: "error" }), "error-id");

  assert.deepEqual(hotToast.blank.mock.calls[0], [
    "Info",
    {
      duration: DEFAULT_TOAST_DURATION,
      ariaProps: { role: "status", "aria-live": "polite" },
    },
  ]);
  assert.deepEqual(hotToast.success.mock.calls[0], [
    "Saved",
    {
      duration: DEFAULT_TOAST_DURATION,
      ariaProps: { role: "status", "aria-live": "polite" },
    },
  ]);
  assert.deepEqual(hotToast.error.mock.calls[0], [
    "Could not save",
    {
      duration: DEFAULT_ERROR_TOAST_DURATION,
      ariaProps: { role: "alert", "aria-live": "assertive" },
    },
  ]);
});

test("toast preserves an explicit duration", () => {
  toast("Custom", { tone: "error", duration: 1234 });
  assert.deepEqual(hotToast.error.mock.calls[0], [
    "Custom",
    {
      duration: 1234,
      ariaProps: { role: "alert", "aria-live": "assertive" },
    },
  ]);
});
