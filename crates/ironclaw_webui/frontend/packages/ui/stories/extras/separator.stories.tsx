import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Separator } from "../../src/extras/separator";

const meta: Meta = { title: "Extras/Separator" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <div className="w-72">
      <div className="text-ui font-semibold text-[var(--v2-text-strong)]">
        IronClaw UI
      </div>
      <p className="text-ui-sm text-[var(--v2-text-muted)]">
        Token-driven component kit.
      </p>
      <Separator className="my-3" />
      <div className="flex h-5 items-center gap-3 text-ui text-[var(--v2-text)]">
        <span>Docs</span>
        <Separator orientation="vertical" />
        <span>Source</span>
        <Separator orientation="vertical" />
        <span>Storybook</span>
      </div>
    </div>
  ),
};
