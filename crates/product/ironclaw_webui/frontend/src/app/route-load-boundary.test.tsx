// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter, useLocation, useNavigate } from "react-router";
import "../i18n/en";
import { I18nProvider } from "../lib/i18n";
import {
  RouteErrorBoundary,
  RouteLoadBoundary,
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

test("healthy route navigation preserves state below the load boundary", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let mounts = 0;
  let unmounts = 0;

  function StatefulRoute() {
    const location = useLocation();
    const navigate = useNavigate();
    const [count, setCount] = React.useState(0);

    React.useEffect(() => {
      mounts += 1;
      return () => {
        unmounts += 1;
      };
    }, []);

    return (
      <>
        <span data-testid="route-state">{location.pathname}:{count}</span>
        <button type="button" onClick={() => setCount((current) => current + 1)}>
          Increment
        </button>
        <button type="button" onClick={() => navigate("/second")}>
          Navigate
        </button>
      </>
    );
  }

  try {
    act(() => {
      root.render(
        <MemoryRouter initialEntries={["/first"]}>
          <I18nProvider>
            <RouteLoadBoundary>
              <StatefulRoute />
            </RouteLoadBoundary>
          </I18nProvider>
        </MemoryRouter>,
      );
    });

    const buttons = container.querySelectorAll("button");
    act(() => buttons[0]?.click());
    assert.equal(container.querySelector('[data-testid="route-state"]')?.textContent, "/first:1");

    act(() => buttons[1]?.click());
    assert.equal(container.querySelector('[data-testid="route-state"]')?.textContent, "/second:1");
    assert.equal(mounts, 1);
    assert.equal(unmounts, 0);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});

test("route navigation clears a prior load failure without remounting healthy routes", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const originalConsoleError = console.error;
  console.error = () => {};

  function RouteContent() {
    const location = useLocation();
    if (location.pathname === "/broken") {
      throw new Error("private chunk failure details");
    }
    return <div data-testid="healthy-route">Healthy route</div>;
  }

  function Navigation() {
    const navigate = useNavigate();
    return (
      <button type="button" onClick={() => navigate("/healthy")}>
        Navigate
      </button>
    );
  }

  try {
    act(() => {
      root.render(
        <MemoryRouter initialEntries={["/broken"]}>
          <I18nProvider>
            <Navigation />
            <RouteLoadBoundary>
              <RouteContent />
            </RouteLoadBoundary>
          </I18nProvider>
        </MemoryRouter>,
      );
    });

    assert.ok(container.querySelector('[role="alert"]'));
    act(() => container.querySelector("button")?.click());
    assert.equal(
      container.querySelector('[data-testid="healthy-route"]')?.textContent,
      "Healthy route",
    );
    assert.equal(container.querySelector('[role="alert"]'), null);
  } finally {
    act(() => root.unmount());
    container.remove();
    console.error = originalConsoleError;
  }
});
