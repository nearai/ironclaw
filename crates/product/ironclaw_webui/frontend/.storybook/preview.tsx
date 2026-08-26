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
  } catch (storageError) {
    // Non-fatal by design: the `data-theme` write above is what app.css keys
    // its token sets off, so the story is already themed correctly whether or
    // not this succeeds. The write only keeps `useInterfaceTheme` in sync for a
    // story that mounts it, and it can fail legitimately (a sandboxed iframe,
    // or a browser blocking storage in a third-party frame). Report the cause
    // rather than discarding it, so a real storage fault is diagnosable instead
    // of silently looking like a successful persist.
    console.warn(
      `[storybook] theme "${theme}" applied but not persisted to localStorage`,
      storageError,
    );
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
      // 'error' - accessibility violations fail `pnpm test:storybook`.
      // 'todo'  - show violations in the test UI only (does not fail).
      // 'off'   - skip a11y checks entirely.
      // The catalog holds itself to the accessibility bar, so violations are a
      // hard failure; add a narrow, documented per-story exclusion only for a
      // genuine known exception.
      test: "error",
      config: {
        // KNOWN EXCEPTION — color-contrast is disabled catalog-wide because the
        // current `--v2-*` token palette has documented AA shortfalls on the
        // faint/muted text tokens (e.g. --v2-text-faint at 11px lands at ~4.47
        // vs the 4.5 threshold). Fixing that means changing token *values*,
        // which is Phase 3 (theme/reskin) of the design-system epic, not the
        // Phase 1 Storybook wiring. Every OTHER a11y rule is enforced as a hard
        // error so new regressions (missing names, invalid ARIA, roles) fail
        // the suite today; Phase 3 re-enables this rule once the palette meets
        // AA. See docs/internal/reborn/design-system/.
        rules: [{ id: "color-contrast", enabled: false }],
      },
    },
  },
};

export default preview;
