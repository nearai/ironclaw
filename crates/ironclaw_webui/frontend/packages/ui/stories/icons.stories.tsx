import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Icon, ICON_NAMES } from "../src/icons/icon";

const meta: Meta = { title: "Icons/Overview" };
export default meta;

export const Icons: StoryObj = {
  render: () => (
    <div className="grid grid-cols-6 gap-3">
      {ICON_NAMES.map((name) => (
        <div
          key={name}
          className="flex flex-col items-center gap-1.5 rounded-[10px] border border-[var(--v2-panel-border)] p-3"
        >
          <Icon name={name} className="h-5 w-5 text-[var(--v2-text)]" />
          <span className="font-mono text-[10px] text-[var(--v2-text-faint)]">{name}</span>
        </div>
      ))}
    </div>
  ),
};
