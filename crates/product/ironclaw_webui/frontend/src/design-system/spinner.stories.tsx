import type { Meta, StoryObj } from "@storybook/react-vite";

import { Spinner } from "./spinner";

const meta = {
  title: "Primitives/Spinner",
  component: Spinner,
  tags: ["ai-generated"],
} satisfies Meta<typeof Spinner>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Sizes: Story = {
  render: () => (
    <div className="flex items-center gap-4 text-[var(--v2-accent-text)]">
      <Spinner className="h-4 w-4" />
      <Spinner className="h-6 w-6" />
      <Spinner className="h-8 w-8" />
    </div>
  ),
};

export const OnAccent: Story = {
  render: () => (
    <div className="text-[var(--v2-accent-text)]">
      <Spinner className="h-6 w-6" />
    </div>
  ),
};
