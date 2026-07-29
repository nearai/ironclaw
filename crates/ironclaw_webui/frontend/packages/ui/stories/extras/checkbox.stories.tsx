import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Checkbox } from "../../src/extras/checkbox";
import { Label } from "../../src/components/input";

const meta: Meta = { title: "Extras/Checkbox" };
export default meta;

type Story = StoryObj;

function ControlledCheckbox() {
  const [checked, setChecked] = React.useState(true);
  return (
    <div className="flex items-center gap-2">
      <Checkbox
        id="cb-controlled"
        checked={checked}
        onCheckedChange={(next) => setChecked(next === true)}
      />
      <Label htmlFor="cb-controlled" className="cursor-pointer">
        Email me run summaries
      </Label>
    </div>
  );
}

export const Default: Story = { render: () => <ControlledCheckbox /> };

export const States: Story = {
  render: () => (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <Checkbox aria-label="Unchecked" />
        <span className="text-ui text-[var(--v2-text)]">Unchecked</span>
      </div>
      <div className="flex items-center gap-2">
        <Checkbox defaultChecked aria-label="Checked" />
        <span className="text-ui text-[var(--v2-text)]">Checked</span>
      </div>
      <div className="flex items-center gap-2">
        <Checkbox checked="indeterminate" aria-label="Indeterminate" />
        <span className="text-ui text-[var(--v2-text)]">Indeterminate</span>
      </div>
      <div className="flex items-center gap-2">
        <Checkbox disabled defaultChecked aria-label="Disabled" />
        <span className="text-ui text-[var(--v2-text-faint)]">Disabled</span>
      </div>
    </div>
  ),
};
