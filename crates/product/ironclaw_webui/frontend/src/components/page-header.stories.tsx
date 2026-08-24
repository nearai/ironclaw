import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

import { withRouter } from "../test-support/storybook-decorators";
import { PageHeader } from "./page-header";

const THREADS_STATE = {
  threads: [{ id: "t1", title: "Deploy pipeline audit" }],
  activeThreadId: "t1",
};

const emptyNotifications = () => ({
  messages: [],
  unreadIds: new Set<string>(),
  hasUnread: false,
  unreadCount: 0,
  dismissMessage: fn(),
});

const meta = {
  title: "Components/PageHeader",
  component: PageHeader,
  decorators: [
    // PageHeader's sidebar-toggle button carries `aria-controls="gateway-sidebar"`,
    // whose target lives in GatewayLayout. In this isolated story that element
    // is absent, so render a hidden stand-in with the same id to keep the ARIA
    // reference valid (real app resolves it against the live sidebar).
    (Story) => (
      <>
        <div id="gateway-sidebar" hidden />
        <Story />
      </>
    ),
    withRouter("/chat/t1"),
  ],
  args: {
    threadsState: THREADS_STATE,
    notificationsState: emptyNotifications(),
    onToggleSidebar: fn(),
    sidebarOpen: true,
  },
  tags: ["ai-generated"],
} satisfies Meta<typeof PageHeader>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const WithUnread: Story = {
  args: {
    notificationsState: {
      messages: [
        { id: "n1", title: "Run needs approval", body: "Approve to continue.", timeLabel: "2m", href: "/chat/t1", icon: "shield" },
      ],
      unreadIds: new Set(["n1"]),
      hasUnread: true,
      unreadCount: 2,
      dismissMessage: fn(),
    },
  },
};

export const SidebarCollapsed: Story = { args: { sidebarOpen: false } };
