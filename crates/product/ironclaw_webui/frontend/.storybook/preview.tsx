import type { Preview, Decorator } from "@storybook/react-vite";

// The real application stylesheet: Tailwind v4 entry + design tokens. Importing
// it here is what makes the `--v2-*` CSS variables and compiled utilities that
// every primitive relies on resolve inside stories.
import "../src/styles/app.css";

// Register the eagerly-bundled English i18n pack so `useT()` resolves real
// strings in stories (mirrors src/main.tsx). Modal, ConfirmDialog and the
// Components stories rely on this rather than shipping their own I18nProvider.
import "../src/i18n/en";

const THEME_STORAGE_KEY = "ironclaw:v2-theme";

/**
 * Mirrors the app's index.html bootstrap: the theme is expressed as
 * `data-theme` on <html>, which is what app.css keys its light/dark token sets
 * off of. Seeding localStorage keeps `useInterfaceTheme` in sync if a story
 * mounts it.
 */
const withTheme: Decorator = (Story, context) => {
  const theme = context.globals.theme === "light" ? "light" : "dark";
  document.documentElement.dataset.theme = theme;
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, theme);
  } catch {
    // Storybook runs in a sandboxed iframe; ignore storage failures.
  }
  return (
    <div className="min-h-[100dvh] bg-[var(--v2-canvas)] p-6 text-[var(--v2-text)]">
      <Story />
    </div>
  );
};

const preview: Preview = {
  decorators: [withTheme],
  globalTypes: {
    theme: {
      description: "App color theme (data-theme on <html>)",
      toolbar: {
        title: "Theme",
        icon: "circlehollow",
        items: [
          { value: "dark", title: "Dark" },
          { value: "light", title: "Light" },
        ],
        dynamicTitle: true,
      },
    },
  },
  initialGlobals: {
    theme: "dark",
  },
  parameters: {
    // Sidebar category order (matches the 5 requested groups).
    options: {
      storySort: {
        order: ["Primitives", "Components", "Composites", "Icons", "Tokens"],
      },
    },
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },

    a11y: {
      // 'todo' - show a11y violations in the test UI only
      // 'error' - fail CI on a11y violations
      // 'off' - skip a11y checks entirely
      test: "todo",
    },
  },
};

export default preview;
