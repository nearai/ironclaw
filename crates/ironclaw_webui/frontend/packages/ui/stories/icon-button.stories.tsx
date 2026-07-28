import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { IconButton } from "../src/components/icon-button";
import { Icon } from "../src/primitives/icon";

const meta: Meta<typeof IconButton> = {
  title: "Components/IconButton",
  component: IconButton,
  args: {
    "aria-label": "Notifications",
    children: <Icon name="bell" className="h-4 w-4" />,
  },
};
export default meta;

type Story = StoryObj<typeof IconButton>;

export const Ghost: Story = {};
export const Active: Story = { args: { active: true } };
export const AsAnchor: Story = {
  args: {
    as: "a",
    href: "https://example.com",
    "aria-label": "Docs",
    children: <Icon name="bookOpen" className="h-4 w-4" />,
  },
};
export const PlainWithCustomColors: Story = {
  name: "Plain (custom colors)",
  args: {
    variant: "plain",
    "aria-label": "Attestation",
    className:
      "border border-[color-mix(in_srgb,var(--v2-positive-text)_28%,transparent)] " +
      "bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",
    children: <Icon name="shield" className="h-4 w-4" />,
  },
};

export const HeaderRow: Story = {
  render: () => (
    <div className="flex items-center gap-1">
      <IconButton aria-label="Toggle sidebar"><Icon name="list" className="h-4 w-4" /></IconButton>
      <IconButton aria-label="Notifications"><Icon name="bell" className="h-4 w-4" /></IconButton>
      <IconButton aria-label="Logs" active><Icon name="terminal" className="h-4 w-4" /></IconButton>
      <IconButton aria-label="Docs"><Icon name="bookOpen" className="h-4 w-4" /></IconButton>
    </div>
  ),
};
