// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import "../i18n/en";
import { I18nProvider } from "../lib/i18n";
import {
  RouteErrorBoundary,
  RouteLoadError,
  RouteLoading,
} from "./route-load-boundary";

function renderWithI18n(element: React.ReactNode) {
  return renderToStaticMarkup(<I18nProvider>{element}</I18nProvider>);
}

test("route loading state is announced without rendering business-page content", () => {
  const html = renderWithI18n(<RouteLoading />);

  assert.match(html, /role="status"/);
  assert.match(html, /aria-live="polite"/);
  assert.match(html, /Loading page/);
});

test("route load failure offers a page reload recovery action", () => {
  const html = renderWithI18n(<RouteLoadError onRetry={() => {}} />);

  assert.match(html, /role="alert"/);
  assert.match(html, /This page couldn&#x27;t be loaded/);
  assert.match(html, /Reload page/);
});

test("route error boundary switches to its sanitized fallback", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let reloads = 0;
  const originalConsoleError = console.error;
  console.error = () => {};

  function ThrowingRoute(): React.ReactNode {
    throw new Error("private chunk failure details");
  }

  try {
    act(() => {
      root.render(
        <I18nProvider>
          <RouteErrorBoundary
            fallback={<RouteLoadError onRetry={() => { reloads += 1; }} />}
          >
            <ThrowingRoute />
          </RouteErrorBoundary>
        </I18nProvider>,
      );
    });

    const alert = container.querySelector('[role="alert"]');
    assert.ok(alert);
    assert.match(alert.textContent ?? "", /This page couldn't be loaded/);
    assert.doesNotMatch(alert.textContent ?? "", /private chunk failure details/);

    const reload = alert.querySelector("button");
    assert.ok(reload);
    assert.equal(reload.textContent, "Reload page");
    act(() => reload.click());
    assert.equal(reloads, 1);
  } finally {
    act(() => root.unmount());
    container.remove();
    console.error = originalConsoleError;
  }
});
