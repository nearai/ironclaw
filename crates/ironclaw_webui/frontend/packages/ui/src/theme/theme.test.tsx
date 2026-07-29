// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { useInterfaceTheme } from "./theme";

test("theme remounts from the live selection instead of the bootstrap snapshot", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const originalInitialTheme = window.__IRONCLAW_INITIAL_THEME__;
  const originalDataTheme = document.documentElement.dataset.theme;
  const originalStoredTheme = window.localStorage.getItem("ironclaw:v2-theme");

  function ThemeHarness() {
    const { theme, setTheme } = useInterfaceTheme();
    return (
      <button type="button" onClick={() => setTheme("dark")}>
        {theme}
      </button>
    );
  }

  try {
    window.__IRONCLAW_INITIAL_THEME__ = "light";
    document.documentElement.dataset.theme = "light";
    window.localStorage.setItem("ironclaw:v2-theme", "light");

    const firstRoot = createRoot(container);
    act(() => firstRoot.render(<ThemeHarness />));
    act(() => container.querySelector("button")?.click());
    assert.equal(container.textContent, "dark");
    assert.equal(document.documentElement.dataset.theme, "dark");
    assert.equal(window.localStorage.getItem("ironclaw:v2-theme"), "dark");
    act(() => firstRoot.unmount());

    const secondRoot = createRoot(container);
    act(() => secondRoot.render(<ThemeHarness />));
    assert.equal(container.textContent, "dark");
    act(() => secondRoot.unmount());
  } finally {
    if (originalInitialTheme === undefined) {
      delete window.__IRONCLAW_INITIAL_THEME__;
    } else {
      window.__IRONCLAW_INITIAL_THEME__ = originalInitialTheme;
    }
    if (originalDataTheme === undefined) {
      delete document.documentElement.dataset.theme;
    } else {
      document.documentElement.dataset.theme = originalDataTheme;
    }
    if (originalStoredTheme === null) {
      window.localStorage.removeItem("ironclaw:v2-theme");
    } else {
      window.localStorage.setItem("ironclaw:v2-theme", originalStoredTheme);
    }
    container.remove();
  }
});
