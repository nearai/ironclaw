import type { Meta, StoryObj } from "@storybook/react-vite";

import { FlowList } from "./primitives";

const meta = {
  title: "Composites/FlowList",
  component: FlowList,
  args: {
    items: [
      {
        title: "Connect a channel",
        description: "Link a chat app or email so the agent can reach you.",
      },
      {
        title: "Install extensions",
        description: "Grant the tools the agent may call on your behalf.",
      },
      { title: "Start a thread", description: "Ask a question or hand off a task." },
    ],
  },
  tags: ["ai-generated"],
} satisfies Meta<typeof FlowList>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
