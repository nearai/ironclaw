import { React } from "../lib/html.js";

const THEME_STORAGE_KEY = "ironclaw:v2-theme";

function getInitialTheme() {
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
  const [theme, setTheme] = React.useState(getInitialTheme);

  React.useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, theme);
    } catch (_) {}
  }, [theme]);

  const toggleTheme = React.useCallback(() => {
    setTheme((current) => (current === "dark" ? "light" : "dark"));
  }, []);

  return { theme, toggleTheme };
}
