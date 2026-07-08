import { describe, expect, test } from "vitest";

import {
  formatCredit,
  formatTimestamp,
  openAccountLoginLink,
  tracesSectionMode,
} from "./trace-commons-tab";

const TRACE = { submission_id: "s1", status: "accepted" };

describe("tracesSectionMode", () => {
  test("traces load error always surfaces, never hides behind an empty state", () => {
    expect(tracesSectionMode({ isError: true, enrolled: true, traces: [TRACE] })).toBe("error");
    // Error wins even for unenrolled/empty views — a backend failure must not
    // render as "no traces".
    expect(tracesSectionMode({ isError: true, enrolled: false, traces: [] })).toBe("error");
  });

  test("enrolled contributor with traces renders the trace list", () => {
    expect(tracesSectionMode({ isError: false, enrolled: true, traces: [TRACE] })).toBe("list");
  });

  test("section hides for empty or unenrolled states", () => {
    expect(tracesSectionMode({ isError: false, enrolled: true, traces: [] })).toBe("hidden");
    expect(tracesSectionMode({ isError: false, enrolled: false, traces: [TRACE] })).toBe("hidden");
    expect(tracesSectionMode({ isError: false, enrolled: false, traces: undefined })).toBe(
      "hidden"
    );
  });
});

test("credit and timestamp formatting used by trace rows", () => {
  expect(formatCredit(1)).toBe("1.00");
  expect(formatCredit("2.5")).toBe("2.50");
  expect(formatCredit(null)).toBe("0.00");
  expect(formatCredit("not-a-number")).toBe("0.00");

  const t = (key: string) => key;
  expect(formatTimestamp(null, t)).toBe("traceCommons.never");
  expect(formatTimestamp("garbage", t)).toBe("traceCommons.never");
  expect(formatTimestamp("2026-06-25T00:00:00Z", t)).not.toBe("traceCommons.never");
});

// Fake window.open that captures EVERY argument the production caller passes
// (url, target, features) — a partial mock would hide a caller that drops the
// noopener hardening — plus the navigations assigned to the opened handle.
function fakeOpener() {
  const calls: Array<{ url: string; target: string; features: string }> = [];
  const windows: Array<{ closed: boolean; location: string | null; close: () => void }> = [];
  return {
    calls,
    windows,
    open: (url: string, target: string, features: string) => {
      calls.push({ url, target, features });
      const win = {
        closed: false,
        location: null as string | null,
        close() {
          this.closed = true;
        },
      };
      windows.push(win);
      return win;
    },
  };
}

describe("openAccountLoginLink", () => {
  test("opens a blank tab synchronously, then navigates it to the minted URL", async () => {
    const opener = fakeOpener();
    const result = await openAccountLoginLink({
      mint: async () => ({
        minted: true,
        enrolled: true,
        url: "https://commons.example/a?code=1",
      }),
      open: opener.open,
    });

    expect(result.status).toBe("opened");
    // The tab must be opened BEFORE the async mint resolves (popup-blocker
    // attribution) with the full hardened argument set.
    expect(opener.calls).toEqual([
      { url: "about:blank", target: "_blank", features: "noopener,noreferrer" },
    ]);
    expect(opener.windows[0].location).toBe("https://commons.example/a?code=1");
    expect(opener.windows[0].closed).toBe(false);
  });

  test("unenrolled mint closes the placeholder tab and reports unavailable", async () => {
    const opener = fakeOpener();
    const result = await openAccountLoginLink({
      mint: async () => ({ minted: false, enrolled: false }),
      open: opener.open,
    });
    expect(result.status).toBe("unavailable");
    expect(opener.windows[0].closed).toBe(true);
    expect(opener.windows[0].location).toBeNull();
  });

  test("mint failure closes the placeholder tab and reports error", async () => {
    const opener = fakeOpener();
    const result = await openAccountLoginLink({
      mint: async () => {
        throw new Error("500");
      },
      open: opener.open,
    });
    expect(result.status).toBe("error");
    expect(opener.windows[0].closed).toBe(true);
  });

  test("popup-blocked open reports blocked without navigating", async () => {
    const result = await openAccountLoginLink({
      mint: async () => ({ minted: true, enrolled: true, url: "https://commons.example/a" }),
      open: () => null,
    });
    expect(result.status).toBe("blocked");
  });
});
