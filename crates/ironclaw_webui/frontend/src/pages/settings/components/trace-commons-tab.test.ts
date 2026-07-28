import { describe, expect, test } from "vitest";

import {
  formatCredit,
  formatTimestamp,
  isSafeLoginLinkUrl,
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

// Fake window.open that captures EVERY argument the production caller passes —
// a partial mock would hide a caller reintroducing the `noopener` feature
// (which makes window.open return null and breaks navigation) — plus the
// opener-severing and navigations applied to the opened handle. Mirrors the
// browser contract: passing a `noopener` feature yields a null handle.
function fakeOpener() {
  const calls: Array<{ args: unknown[] }> = [];
  const windows: Array<{
    closed: boolean;
    location: string | null;
    opener: unknown;
    close: () => void;
  }> = [];
  return {
    calls,
    windows,
    open: (...args: unknown[]) => {
      calls.push({ args });
      const features = String(args[2] ?? "");
      if (features.includes("noopener")) {
        // Real browsers return null for noopener opens — the production
        // caller must NOT pass it as a feature or it can never navigate.
        return null;
      }
      const win = {
        closed: false,
        location: null as string | null,
        opener: "parent-window-sentinel" as unknown,
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
  test("opens a blank tab synchronously, severs opener, then navigates it", async () => {
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
    // attribution), WITHOUT a `noopener` feature (which would null the handle
    // and make navigation impossible).
    expect(opener.calls).toEqual([{ args: ["about:blank", "_blank"] }]);
    // Reverse-tabnabbing protection comes from severing the opener manually.
    expect(opener.windows[0].opener).toBeNull();
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

  test("non-web minted URLs are refused before navigation (defense in depth)", async () => {
    // The about:blank tab inherits the WebUI origin — a javascript: URL would
    // execute with WebUI-origin access. The backend origin-pins the URL, but
    // the client refuses independently.
    expect(isSafeLoginLinkUrl("https://commons.example/a")).toBe(true);
    expect(isSafeLoginLinkUrl("http://127.0.0.1:8080/a")).toBe(true);
    expect(isSafeLoginLinkUrl("javascript:alert(1)")).toBe(false);
    expect(isSafeLoginLinkUrl("data:text/html,x")).toBe(false);
    expect(isSafeLoginLinkUrl("/relative/path")).toBe(false);

    const opener = fakeOpener();
    const result = await openAccountLoginLink({
      mint: async () => ({ minted: true, enrolled: true, url: "javascript:alert(1)" }),
      open: opener.open,
    });
    expect(result.status).toBe("unavailable");
    expect(opener.windows[0].closed).toBe(true);
    expect(opener.windows[0].location).toBeNull();
  });

  test("popup-blocked open reports blocked WITHOUT burning a one-time link", async () => {
    let minted = 0;
    const result = await openAccountLoginLink({
      mint: async () => {
        minted += 1;
        return { minted: true, enrolled: true, url: "https://commons.example/a" };
      },
      open: () => null,
    });
    expect(result.status).toBe("blocked");
    // Each mint burns a single-use credential server-side; a blocked popup
    // must short-circuit BEFORE the mint call.
    expect(minted).toBe(0);
  });
});
