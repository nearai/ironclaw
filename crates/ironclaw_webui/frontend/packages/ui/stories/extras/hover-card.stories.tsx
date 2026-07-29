import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { HoverCard, HoverCardContent, HoverCardTrigger } from "../../src/extras/hover-card";
import { Avatar, AvatarFallback } from "../../src/extras/avatar";

const meta: Meta = { title: "Extras/HoverCard" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <HoverCard openDelay={200}>
      <HoverCardTrigger asChild>
        <button
          type="button"
          className="text-ui font-medium text-[var(--v2-accent-text)] underline decoration-dotted underline-offset-4"
        >
          @ironclaw-agent
        </button>
      </HoverCardTrigger>
      <HoverCardContent>
        <div className="flex items-start gap-3">
          <Avatar size="sm"><AvatarFallback>IA</AvatarFallback></Avatar>
          <div className="flex flex-col gap-1">
            <span className="text-ui font-semibold text-[var(--v2-text-strong)]">
              IronClaw Agent
            </span>
            <span className="text-ui-sm text-[var(--v2-text-muted)]">
              Autonomous agent runtime. 128 runs this week, 97% success.
            </span>
          </div>
        </div>
      </HoverCardContent>
    </HoverCard>
  ),
};
