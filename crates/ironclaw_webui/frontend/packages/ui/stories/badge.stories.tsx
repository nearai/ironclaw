import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Badge } from "../src/components/badge";

const meta: Meta<typeof Badge> = {
  title: "Components/Badge",
  component: Badge,
  args: { label: "Badge", tone: "muted" },
  argTypes: {
    tone: {
      control: "select",
      options: [
        "success", "positive", "signal", "warning", "copper",
        "danger", "info", "accent", "muted",
      ],
    },
    size: { control: "select", options: ["sm", "md"] },
  },
};
export default meta;

type Story = StoryObj<typeof Badge>;

export const Muted: Story = {};
export const AllTones: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-2">
      {(["success", "warning", "danger", "info", "accent", "muted"] as const).map((tone) => (
        <Badge key={tone} tone={tone} label={tone} />
      ))}
    </div>
  ),
};
export const Small: Story = { args: { size: "sm", label: "small" } };
export const WithoutDot: Story = { args: { dot: false, label: "no dot" } };
