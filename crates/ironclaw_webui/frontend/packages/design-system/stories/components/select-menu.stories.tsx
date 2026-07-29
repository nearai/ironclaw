import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { SelectMenu } from "../../src/select-menu";

const meta = {
  title: "Components/SelectMenu",
  component: SelectMenu,
  parameters: {
    docs: {
      description: {
        component:
          "Value-picking select on `@radix-ui/react-select` (shadcn Select pattern): " +
          "full keyboard navigation, typeahead, and aria wiring for free. Options can " +
          "carry a `tone` for a status dot; `prefix` labels the closed control. For " +
          "command/action menus use DropdownMenu instead.",
      },
    },
  },
  argTypes: {
    prefix: { control: "text" },
    disabled: { control: "boolean" },
    placeholder: { control: "text" },
    align: { control: "select", options: ["left", "right"] },
  },
  args: { disabled: false },
} satisfies Meta<typeof SelectMenu>;

export default meta;
type Story = StoryObj<typeof meta>;

function StatefulSelect(props: {
  prefix?: string;
  disabled?: boolean;
  placeholder?: string;
  align?: "left" | "right";
  /** Start without a selection so the `placeholder` control is visible. */
  initialValue?: string;
}) {
  const { initialValue = "all", ...rest } = props;
  const [value, setValue] = useState(initialValue);
  return (
    <SelectMenu
      {...rest}
      value={value}
      onChange={setValue}
      options={[
        { value: "all", label: "All statuses" },
        { value: "running", label: "Running", tone: "info" },
        { value: "success", label: "Success", tone: "positive" },
        { value: "failed", label: "Failed", tone: "danger" },
        { value: "paused", label: "Paused", tone: "neutral", disabled: true },
      ]}
    />
  );
}

export const Default: Story = {
  render: (args) => <StatefulSelect {...(args as Record<string, unknown>)} />,
};

export const WithPrefix: Story = {
  args: { prefix: "Status" },
  render: (args) => <StatefulSelect {...(args as Record<string, unknown>)} />,
};

export const Placeholder: Story = {
  args: { placeholder: "Filter by status…" },
  render: (args) => (
    <StatefulSelect initialValue="" {...(args as Record<string, unknown>)} />
  ),
};
