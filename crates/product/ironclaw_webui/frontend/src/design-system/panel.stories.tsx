import type { Meta, StoryObj } from "@storybook/react-vite";

import { Panel } from "./primitives";

const meta = {
  title: "Composites/Panel",
  component: Panel,
  args: { className: "p-5", children: "Panel content" },
  tags: ["ai-generated"],
} satisfies Meta<typeof Panel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Composed: Story = {
  render: () => (
    <Panel className="p-6">
      <h3 className="text-base font-semibold text-[var(--v2-text-strong)]">Section title</h3>
      <p className="mt-2 text-sm text-[var(--v2-text-muted)]">
        Panel is a thin wrapper over Card used across settings and dashboards.
      </p>
    </Panel>
  ),
};
