import type { Meta, StoryObj } from "@storybook/react-vite";

import { Button } from "./button";
import { EmptyPanel } from "./primitives";

const meta = {
  title: "Composites/EmptyPanel",
  component: EmptyPanel,
  args: { title: "No missions yet", description: "Missions you create will appear here." },
  tags: ["ai-generated"],
} satisfies Meta<typeof EmptyPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const WithCta: Story = {
  args: {
    title: "No projects",
    description: "Create your first project to organize threads and missions.",
    children: (
      <Button variant="primary" size="sm">New project</Button>
    ),
  },
};

export const Unboxed: Story = { args: { boxed: false } };
