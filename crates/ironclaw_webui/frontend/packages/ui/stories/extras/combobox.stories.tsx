import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Combobox } from "../../src/extras/combobox";

const meta: Meta = { title: "Extras/Combobox" };
export default meta;

type Story = StoryObj;

const REGIONS = [
  { value: "us-east", label: "US East (N. Virginia)" },
  { value: "us-west", label: "US West (Oregon)" },
  { value: "eu-central", label: "EU Central (Frankfurt)" },
  { value: "eu-west", label: "EU West (Ireland)" },
  { value: "ap-south", label: "Asia Pacific (Mumbai)" },
  { value: "ap-northeast", label: "Asia Pacific (Tokyo)", disabled: true },
];

function Demo(props: { disabled?: boolean }) {
  const [value, setValue] = React.useState<string | null>("eu-central");
  return (
    <div className="h-72">
      <Combobox
        options={REGIONS}
        value={value}
        onChange={setValue}
        placeholder="Select region"
        aria-label="Region"
        {...props}
      />
    </div>
  );
}

export const Default: Story = { render: () => <Demo /> };
export const Disabled: Story = { render: () => <Demo disabled /> };

export const Empty: Story = {
  render: () => (
    <div className="h-72">
      <Combobox
        options={[]}
        emptyMessage="No regions configured"
        placeholder="Select region"
        aria-label="Region"
      />
    </div>
  ),
};
