import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

import { withRouter } from "../test-support/storybook-decorators";
import { SidebarThreads } from "./sidebar-threads";

const THREADS = [
  { id: "t1", title: "Deploy pipeline audit", updated_at: "2026-07-30T14:00:00Z" },
  { id: "t2", title: "Q3 roadmap notes", updated_at: "2026-07-29T09:30:00Z", state: "running" },
  { id: "t3", title: "Incident postmortem", updated_at: "2026-07-28T18:15:00Z", state: "needs_attention" },
  { id: "t4", title: "Billing reconciliation", updated_at: "2026-07-27T11:00:00Z", state: "failed" },
];

const meta = {
  title: "Components/SidebarThreads",
  component: SidebarThreads,
  decorators: [
    withRouter("/chat"),
    (Story) => (
      <div className="flex h-[26rem] w-72 flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] py-2">
        <Story />
      </div>
    ),
  ],
  args: {
    threads: THREADS,
    activeThreadId: "t1",
    onSelect: fn(),
    onDelete: fn(),
    onLoadMore: fn(),
    onNavigate: fn(),
  },
  tags: ["ai-generated"],
} satisfies Meta<typeof SidebarThreads>;

export default meta;
type Story = StoryObj<typeof meta>;

// THREADS carries running / needs-attention / failed states so their dots show.
export const Default: Story = {};
export const Empty: Story = { args: { threads: [] } };
export const LoadMore: Story = { args: { hasMore: true } };
export const LoadMoreError: Story = { args: { hasMore: true, loadMoreError: "boom" } };
