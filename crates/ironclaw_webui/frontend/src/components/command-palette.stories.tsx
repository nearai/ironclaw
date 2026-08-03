import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn } from "storybook/test";

import { withRouter } from "../test-support/storybook-decorators";
import { CommandPalette } from "./command-palette";

const THREADS_STATE = {
  threads: [
    { id: "t_9f2a3b1c", title: "Deploy pipeline audit" },
    { id: "t_5e8d4a2f", title: "Q3 roadmap notes" },
  ],
  activeThreadId: null,
};

const meta = {
  title: "Components/CommandPalette",
  component: CommandPalette,
  decorators: [withRouter("/chat")],
  args: {
    open: true,
    onClose: fn(),
    threadsState: THREADS_STATE,
    onNewChat: fn(),
    onToggleTheme: fn(),
  },
  tags: ["ai-generated"],
} satisfies Meta<typeof CommandPalette>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Open: Story = {};
export const NoThreads: Story = { args: { threadsState: { threads: [], activeThreadId: null } } };

export const Filtering: Story = {
  // Typing filters the command list — prove a thread command survives the filter.
  play: async ({ canvas, userEvent }) => {
    const input = canvas.getByRole("textbox");
    await userEvent.type(input, "roadmap");
    await expect(input).toHaveValue("roadmap");
    await expect(canvas.getByText("Q3 roadmap notes")).toBeVisible();
  },
};
