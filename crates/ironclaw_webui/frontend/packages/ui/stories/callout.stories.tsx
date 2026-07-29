import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Callout } from "../src/components/callout";

const meta: Meta<typeof Callout> = {
  title: "Components/Callout",
  component: Callout,
  args: { children: "Workspace refreshed." },
  argTypes: {
    tone: { control: "select", options: ["info", "success", "danger"] },
  },
};
export default meta;

type Story = StoryObj<typeof Callout>;

export const Info: Story = { render: (args) => <div className="w-[28rem]"><Callout {...args} /></div> };
export const Tones: Story = {
  render: () => (
    <div className="grid w-[28rem] gap-3">
      <Callout tone="info">Something informational happened.</Callout>
      <Callout tone="success">Saved successfully.</Callout>
      <Callout tone="danger" role="alert">Failed to reach the gateway.</Callout>
    </div>
  ),
};
export const Dismissible: Story = {
  render: () => (
    <div className="w-[28rem]">
      <Callout tone="success" onDismiss={() => {}} dismissLabel="Dismiss">
        Automation created.
      </Callout>
    </div>
  ),
};
