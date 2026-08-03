import type { Meta, StoryObj } from "@storybook/react-vite";

import { SubLabel } from "./primitives";

const meta = {
  title: "Composites/SubLabel",
  component: SubLabel,
  args: { children: "Danger zone" },
  tags: ["ai-generated"],
} satisfies Meta<typeof SubLabel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
