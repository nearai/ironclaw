import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

import { withRouter } from "../test-support/storybook-decorators";
import { SidebarNav } from "./sidebar-nav";

const meta = {
  title: "Components/SidebarNav",
  component: SidebarNav,
  decorators: [withRouter("/chat")],
  args: { onNewChat: fn(), isCreating: false, isAdmin: true, onNavigate: fn() },
  parameters: { layout: "centered" },
  tags: ["ai-generated"],
} satisfies Meta<typeof SidebarNav>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const Creating: Story = { args: { isCreating: true } };
export const NonAdmin: Story = { args: { isAdmin: false } };
