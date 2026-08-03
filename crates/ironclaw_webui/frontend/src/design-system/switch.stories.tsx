import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { expect } from "storybook/test";

import { Switch } from "./switch";

type ToggleProps = {
  checked?: boolean;
  disabled?: boolean;
  size?: "sm" | "md";
  "aria-label"?: string;
};

/**
 * Switch is a controlled component (`checked` + `onChange`). This thin wrapper
 * owns the state so the toggle is interactive in the Storybook canvas, and it
 * always supplies the required accessible name to the underlying Switch.
 */
function Toggle({
  checked = false,
  "aria-label": ariaLabel = "Toggle setting",
  ...rest
}: ToggleProps) {
  const [on, setOn] = useState(checked);
  return <Switch {...rest} aria-label={ariaLabel} checked={on} onChange={setOn} />;
}

const meta = {
  title: "Primitives/Switch",
  component: Toggle,
  args: { "aria-label": "Enable notifications", checked: false, size: "md" },
  tags: ["ai-generated"],
} satisfies Meta<typeof Toggle>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Off: Story = {};
export const On: Story = { args: { checked: true } };
export const Small: Story = { args: { size: "sm" } };
export const Disabled: Story = { args: { disabled: true } };
export const DisabledOn: Story = { args: { disabled: true, checked: true } };

export const Toggles: Story = {
  // A switch's whole job is the state transition — assert the click flips
  // aria-checked, which is what assistive tech reads.
  play: async ({ canvas, userEvent }) => {
    const toggle = canvas.getByRole("switch", { name: /enable notifications/i });
    await expect(toggle).toHaveAttribute("aria-checked", "false");
    await userEvent.click(toggle);
    await expect(toggle).toHaveAttribute("aria-checked", "true");
  },
};
