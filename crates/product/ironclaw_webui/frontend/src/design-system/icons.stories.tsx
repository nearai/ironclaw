import type { Meta, StoryObj } from "@storybook/react-vite";

import { Icon, iconNames } from "./icons";

const meta = {
  title: "Icons/Icon",
  component: Icon,
  args: { name: "spark", className: "h-6 w-6 text-[var(--v2-text-strong)]", strokeWidth: 1.7 },
  argTypes: {
    name: { control: "select", options: iconNames },
    strokeWidth: { control: { type: "range", min: 1, max: 3, step: 0.1 } },
  },
  tags: ["ai-generated"],
} satisfies Meta<typeof Icon>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Playground: Story = {};

export const Gallery: Story = {
  render: () => (
    <div className="grid grid-cols-3 gap-3 sm:grid-cols-4 md:grid-cols-6">
      {iconNames.map((name) => (
        <div
          key={name}
          className="flex flex-col items-center gap-2 rounded-[12px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3 text-[var(--v2-text-strong)]"
        >
          <Icon name={name} className="h-6 w-6" />
          <span className="font-mono text-[0.625rem] text-[var(--v2-text-muted)]">{name}</span>
        </div>
      ))}
    </div>
  ),
};
