import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "../../src/extras/tooltip";
import { IconButton } from "../../src/components/icon-button";
import { Button } from "../../src/components/button";
import { Icon } from "../../src/primitives/icon";

const meta: Meta = { title: "Extras/Tooltip" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <TooltipProvider delayDuration={200}>
      <Tooltip>
        <TooltipTrigger asChild>
          <IconButton aria-label="Settings">
            <Icon name="settings" className="h-4 w-4" />
          </IconButton>
        </TooltipTrigger>
        <TooltipContent>Open settings</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  ),
};

export const Sides: Story = {
  render: () => (
    <TooltipProvider delayDuration={0}>
      <div className="flex gap-3">
        {(["top", "right", "bottom", "left"] as const).map((side) => (
          <Tooltip key={side}>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="sm">{side}</Button>
            </TooltipTrigger>
            <TooltipContent side={side}>Tooltip on {side}</TooltipContent>
          </Tooltip>
        ))}
      </div>
    </TooltipProvider>
  ),
};
