import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Switch } from "../src/components/switch";
import { Label } from "../src/components/input";

const meta: Meta = { title: "Components/Switch" };
export default meta;

type Story = StoryObj;

function Demo() {
  const [checked, setChecked] = React.useState(true);
  return (
    <div className="flex items-center gap-2.5">
      <Switch id="sw-notifications" checked={checked} onCheckedChange={setChecked} />
      <Label htmlFor="sw-notifications" className="cursor-pointer">
        Run notifications
      </Label>
    </div>
  );
}

export const Default: Story = { render: () => <Demo /> };

export const States: Story = {
  render: () => (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2.5">
        <Switch aria-label="Off" />
        <span className="text-ui text-[var(--v2-text)]">Off</span>
      </div>
      <div className="flex items-center gap-2.5">
        <Switch defaultChecked aria-label="On" />
        <span className="text-ui text-[var(--v2-text)]">On</span>
      </div>
      <div className="flex items-center gap-2.5">
        <Switch disabled aria-label="Disabled off" />
        <span className="text-ui text-[var(--v2-text-faint)]">Disabled</span>
      </div>
      <div className="flex items-center gap-2.5">
        <Switch disabled defaultChecked aria-label="Disabled on" />
        <span className="text-ui text-[var(--v2-text-faint)]">Disabled, on</span>
      </div>
    </div>
  ),
};
