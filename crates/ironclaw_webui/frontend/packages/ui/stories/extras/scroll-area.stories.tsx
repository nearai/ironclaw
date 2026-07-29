import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { ScrollArea, ScrollBar } from "../../src/extras/scroll-area";

const meta: Meta = { title: "Extras/ScrollArea" };
export default meta;

type Story = StoryObj;

export const Vertical: Story = {
  render: () => (
    <ScrollArea className="h-56 w-64 rounded-[10px] border border-[var(--v2-panel-border)]">
      <div className="flex flex-col gap-1 p-3">
        {Array.from({ length: 30 }, (_row, index) => (
          <div
            key={index}
            className="rounded-[7px] px-2.5 py-1.5 text-ui text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]"
          >
            Log line {index + 1}
          </div>
        ))}
      </div>
    </ScrollArea>
  ),
};

export const Horizontal: Story = {
  render: () => (
    <ScrollArea className="w-80 rounded-[10px] border border-[var(--v2-panel-border)]">
      <div className="flex w-max gap-2 p-3">
        {Array.from({ length: 12 }, (_card, index) => (
          <div
            key={index}
            className="grid h-24 w-32 shrink-0 place-items-center rounded-[10px] bg-[var(--v2-surface-soft)] text-ui text-[var(--v2-text-muted)]"
          >
            Card {index + 1}
          </div>
        ))}
      </div>
      <ScrollBar orientation="horizontal" />
    </ScrollArea>
  ),
};
