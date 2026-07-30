import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { SegmentedControl } from "../src/composites/segmented-control";

const meta: Meta = { title: "Composites/SegmentedControl" };
export default meta;

type Story = StoryObj;

function Demo() {
  const [value, setValue] = React.useState("all");
  return (
    <SegmentedControl
      label="Filter automations"
      value={value}
      onChange={setValue}
      options={[
        { value: "all", label: "All" },
        { value: "active", label: "Active" },
        { value: "running", label: "Running" },
        { value: "failures", label: "Failures" },
        { value: "completed", label: "Completed", disabled: true },
      ]}
    />
  );
}

export const Default: Story = { render: () => <Demo /> };
