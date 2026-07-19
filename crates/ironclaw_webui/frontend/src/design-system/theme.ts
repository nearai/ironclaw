import React from "react";

const THEME_STORAGE_KEY = "ironclaw:v2-theme";

export type InterfaceTheme = "light" | "dark";

function getInitialTheme(): InterfaceTheme {
  try {
    if (window.__IRONCLAW_INITIAL_THEME__ === "light" || window.__IRONCLAW_INITIAL_THEME__ === "dark") {
      return window.__IRONCLAW_INITIAL_THEME__;
    }
    const current = document.documentElement.dataset.theme;
    if (current === "light" || current === "dark") return current;
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    if (stored === "light" || stored === "dark") return stored;
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
