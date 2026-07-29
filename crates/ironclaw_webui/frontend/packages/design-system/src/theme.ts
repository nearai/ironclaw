import React from "react";

declare global {
  interface Window {
    /** Optional server-injected initial theme (set by the host shell before hydration). */
    __IRONCLAW_INITIAL_THEME__?: "light" | "dark";
  }
}

const THEME_STORAGE_KEY = "ironclaw:v2-theme";

export type InterfaceTheme = "light" | "dark";

function getInitialTheme(): InterfaceTheme {
  try {
    // The bootstrap snapshot prevents first-paint flicker, but it becomes stale
    // after an in-app theme change. Prefer the live/persisted selection on remount.
    const current = document.documentElement.dataset.theme;
    if (current === "light" || current === "dark") return current;
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    if (stored === "light" || stored === "dark") return stored;
    if (window.__IRONCLAW_INITIAL_THEME__ === "light" || window.__IRONCLAW_INITIAL_THEME__ === "dark") {
      return window.__IRONCLAW_INITIAL_THEME__;
    }
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  } catch (_) {
    return "light";
  }
}

export function useInterfaceTheme() {
  const [theme, setThemeState] = React.useState(getInitialTheme);

  React.useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, theme);
    } catch (_) {}
  }, [theme]);

  const toggleTheme = React.useCallback(() => {
    setThemeState((current) => (current === "dark" ? "light" : "dark"));
  }, []);

  const setTheme = React.useCallback((nextTheme: InterfaceTheme) => {
    if (nextTheme === "light" || nextTheme === "dark") {
      setThemeState(nextTheme);
    }
  }, []);

  return { theme, setTheme, toggleTheme };
}
