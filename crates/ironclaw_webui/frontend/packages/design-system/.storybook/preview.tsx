import type { Preview } from "@storybook/react-vite";
import { withThemeByDataAttribute } from "@storybook/addon-themes";
import { ironclawTheme } from "./ironclaw-theme";
import "./storybook.css";

const preview: Preview = {
  tags: ["autodocs"],
  parameters: {
    layout: "centered",
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
    // Canvas background follows --v2-canvas via the theme decorator;
    // Storybook's own background switcher would fight it.
    backgrounds: { disable: true },
    docs: { toc: true, theme: ironclawTheme },
    options: {
      storySort: {
        order: [
          "Docs",
          ["Getting started", "Voice & copy", "Contributing", "Brand token spec"],
          "Tokens",
          "Components",
          "Compositions",
        ],
      },
    },
  },
  decorators: [
    withThemeByDataAttribute({
      themes: {
        light: "light",
        dark: "dark",
        soft: "soft",
      },
      defaultTheme: "light",
      attributeName: "data-theme",
      parentSelector: "html",
    }),
  ],
};

export default preview;
