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
} satisfies Meta<typeof SelectMenu>;

export default meta;
type Story = StoryObj<typeof meta>;

function StatefulSelect(props: { prefix?: string }) {
  const [value, setValue] = useState("all");
  return (
    <SelectMenu
      value={value}
      onChange={setValue}
      prefix={props.prefix}
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
  render: () => <StatefulSelect />,
};

export const WithPrefix: Story = {
  render: () => <StatefulSelect prefix="Status" />,
};
