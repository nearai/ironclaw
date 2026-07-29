import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { AspectRatio } from "../../src/extras/aspect-ratio";

const meta: Meta = { title: "Extras/AspectRatio" };
export default meta;

type Story = StoryObj;

function Frame({ label }: { label: string }) {
  return (
    <div className="grid h-full w-full place-items-center rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-ui text-[var(--v2-text-muted)]">
      {label}
    </div>
  );
}

export const SixteenByNine: Story = {
  render: () => (
    <div className="w-80">
      <AspectRatio ratio={16 / 9}>
        <Frame label="16 : 9" />
      </AspectRatio>
    </div>
  ),
};

export const Square: Story = {
  render: () => (
    <div className="w-48">
      <AspectRatio ratio={1}>
        <Frame label="1 : 1" />
      </AspectRatio>
    </div>
  ),
};
