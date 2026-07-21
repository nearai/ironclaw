import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Tabs } from "../../src/tabs";

const meta = {
  title: "Components/Tabs",
  component: Tabs,
  parameters: {
    docs: {
      description: {
        component:
          "Underline tab row on `@radix-ui/react-tabs` (arrow-key roving, aria " +
          "wiring) with the shared-layout underline slide. Optional `count` renders " +
          "a per-tab tally; `bordered` adds the full-width baseline rule.",
      },
    },
  },
  argTypes: {
    bordered: { control: "boolean" },
  },
  args: { bordered: false },
} satisfies Meta<typeof Tabs>;

export default meta;
type Story = StoryObj<typeof meta>;

function TabsDemo({ bordered }: { bordered?: boolean }) {
  const [value, setValue] = useState("all");
  return (
    <div className="w-96">
      <Tabs
        ariaLabel="Automation filters"
        bordered={bordered}
        value={value}
        onChange={setValue}
        tabs={[
          { value: "all", label: "All", count: 12 },
          { value: "active", label: "Active", count: 8 },
          { value: "paused", label: "Paused", count: 3 },
          { value: "failed", label: "Failed", count: 1 },
        ]}
      />
      <p className="mt-4 text-sm text-[var(--v2-text-muted)]">
        Showing <span className="text-[var(--v2-text-strong)]">{value}</span> automations.
      </p>
    </div>
  );
}

export const Default: Story = {
  render: (args) => <TabsDemo bordered={args.bordered} />,
};
export const Bordered: Story = {
  args: { bordered: true },
  render: (args) => <TabsDemo bordered={args.bordered} />,
};
