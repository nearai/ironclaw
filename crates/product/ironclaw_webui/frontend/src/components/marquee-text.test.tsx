// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, test, vi } from "vitest";
import { MarqueeText } from "./marquee-text";

const roots = [];
const originalResizeObserver = globalThis.ResizeObserver;

afterEach(() => {
  for (const root of roots.splice(0)) root.unmount();
  vi.restoreAllMocks();
  globalThis.ResizeObserver = originalResizeObserver;
});

async function renderMarquee({ clientWidth, scrollWidth }) {
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(clientWidth);
  vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockReturnValue(scrollWidth);
  globalThis.ResizeObserver = class MockResizeObserver implements ResizeObserver {
    constructor(private readonly callback: ResizeObserverCallback) {}
    observe() {
      this.callback([], this);
    }
    unobserve() {}
    disconnect() {}
  };

  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  roots.push(root);
  await act(async () => {
    root.render(<MarqueeText>Conversation title that may overflow</MarqueeText>);
  });
  return container;
}

test("MarqueeText enables the hover track only when its title overflows", async () => {
  const container = await renderMarquee({ clientWidth: 120, scrollWidth: 260 });
  const marquee = container.querySelector("[data-marquee-overflow]");

  assert.equal(marquee?.getAttribute("data-marquee-overflow"), "true");
  assert.equal(marquee?.querySelector("[aria-hidden='true']")?.textContent, "Conversation title that may overflow");
  assert.match(marquee?.getAttribute("style") || "", /--marquee-distance: 284px/);
});

test("MarqueeText leaves a fitting title static", async () => {
  const container = await renderMarquee({ clientWidth: 260, scrollWidth: 120 });
  const marquee = container.querySelector("[data-marquee-overflow]");

  assert.equal(marquee?.getAttribute("data-marquee-overflow"), "false");
  assert.equal(marquee?.querySelector("[aria-hidden='true']"), null);
});

test("MarqueeText tests restore the environment ResizeObserver", () => {
  assert.equal(globalThis.ResizeObserver, originalResizeObserver);
});
