import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

import { SidebarFooter } from "./sidebar-footer";

const PROFILE = {
  id: "u_ada",
  display_name: "Ada Lovelace",
  email: "ada@ironclaw.dev",
  role: "admin",
};

const meta = {
  title: "Components/SidebarFooter",
  component: SidebarFooter,
  args: { theme: "dark", toggleTheme: fn(), profile: PROFILE, onSignOut: fn() },
  // SidebarFooter is a full-width bar; give it a sidebar-ish frame.
  decorators: [
    (Story) => (
      <div className="w-72 rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)]">
        <Story />
      </div>
    ),
  ],
  tags: ["ai-generated"],
} satisfies Meta<typeof SidebarFooter>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const LightTheme: Story = { args: { theme: "light" } };
export const SessionOnly: Story = { args: { profile: { id: "session" } } };
