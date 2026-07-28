import React from "react";
import type { Decorator, Preview } from "@storybook/react-vite";
import "./storybook.css";

const withTheme: Decorator = (Story, context) => {
  const theme = context.globals.theme === "light" ? "light" : "dark";
  React.useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);
  return (<Story />);
};

const preview: Preview = {
  decorators: [withTheme],
  globalTypes: {
    theme: {
      description: "Color theme",
      toolbar: {
        title: "Theme",
        icon: "mirror",
        items: [
          { value: "light", title: "Light" },
          { value: "dark", title: "Dark" },
        ],
        dynamicTitle: true,
      },
    },
  },
  initialGlobals: { theme: "dark" },
  parameters: {
    layout: "centered",
    backgrounds: { disable: true },
  },
};

export default preview;
