import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

import { withQueryClient, withRouter } from "../test-support/storybook-decorators";
import { Sidebar } from "./sidebar";

const THREADS_STATE = {
  isCreating: false,
  threads: [
    { id: "t1", title: "Deploy pipeline audit", updated_at: "2026-07-30T14:00:00Z" },
    { id: "t2", title: "Q3 roadmap notes", updated_at: "2026-07-29T09:30:00Z", state: "running" },
    { id: "t3", title: "Incident postmortem", updated_at: "2026-07-28T18:15:00Z", state: "needs_attention" },
  ],
  activeThreadId: "t1",
  hasMore: false,
  isLoadingMore: false,
  loadMoreError: null,
  loadMore: fn(),
  setActiveThreadId: fn(),
  deleteThread: fn(),
  createThread: fn(),
};

const PROFILE = { id: "u_ada", display_name: "Ada Lovelace", email: "ada@ironclaw.dev", role: "admin" };

const meta = {
  title: "Components/Sidebar",
  component: Sidebar,
  decorators: [
    withRouter("/chat"),
    // Trace-credits is enrolled so the credits card shows; the sidebar is
    // full-height, so frame it in a fixed-height column.
    withQueryClient((client) =>
      client.setQueryData(["trace-credits"], {
        enrolled: true,
        final_credit: 12.5,
        submissions_accepted: 8,
        submissions_submitted: 10,
        manual_review_hold_count: 0,
      }),
    ),
    (Story) => (
      <div className="flex h-[640px]">
        <Story />
      </div>
    ),
  ],
  args: {
    id: "gateway-sidebar",
    threadsState: THREADS_STATE,
    theme: "dark",
    toggleTheme: fn(),
    profile: PROFILE,
    isAdmin: true,
    rebornProjectsEnabled: false,
    onSignOut: fn(),
    onClose: fn(),
    onNewChat: fn(),
    onSelectThread: fn(),
    onDeleteThread: fn(),
  },
  tags: ["ai-generated"],
} satisfies Meta<typeof Sidebar>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const NonAdmin: Story = { args: { isAdmin: false } };
export const Empty: Story = {
  args: { threadsState: { ...THREADS_STATE, threads: [], activeThreadId: null } },
};
