import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
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
  assert.deepEqual(RouteErrorBoundary.getDerivedStateFromError(), { failed: true });
});
