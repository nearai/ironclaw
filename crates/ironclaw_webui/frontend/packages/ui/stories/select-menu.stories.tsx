import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { SelectMenu } from "../src/components/select-menu";

const OPTIONS = [
  { value: "running", label: "Running", tone: "positive" as const },
  { value: "paused", label: "Paused", tone: "warning" as const },
  { value: "failed", label: "Failed", tone: "danger" as const },
  { value: "archived", label: "Archived", disabled: true },
];

function ControlledSelectMenu(props: { disabled?: boolean; align?: "left" | "right" }) {
  const [value, setValue] = React.useState("running");
  return (
    <SelectMenu
      value={value}
      options={OPTIONS}
      onChange={setValue}
      aria-label="Status"
      {...props}
    />
  );
}

const meta: Meta<typeof SelectMenu> = {
  title: "Components/SelectMenu",
  component: SelectMenu,
};
export default meta;

type Story = StoryObj<typeof SelectMenu>;

export const Default: Story = { render: () => <ControlledSelectMenu /> };
export const LeftAligned: Story = { render: () => <ControlledSelectMenu align="left" /> };
export const Disabled: Story = { render: () => <ControlledSelectMenu disabled /> };
