import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, within } from "storybook/test";

import { withRouter } from "../test-support/storybook-decorators";
import { NotificationCenter } from "./notification-center";

const STATE = {
  messages: [
    {
      id: "n1",
      title: "Run needs approval",
      body: "Approve the deploy to continue.",
      detail: "chat",
      timeLabel: "2m",
      href: "/chat/t1",
      icon: "shield",
    },
    { id: "n2", title: "Job completed", body: "Weekly digest finished.", timeLabel: "1h", icon: "check" },
  ],
  unreadIds: new Set(["n1"]),
  hasUnread: true,
  unreadCount: 1,
  dismissMessage: fn(),
};

const EMPTY_STATE = {
  messages: [],
  unreadIds: new Set<string>(),
  hasUnread: false,
  unreadCount: 0,
  dismissMessage: fn(),
};

const meta = {
  title: "Components/NotificationCenter",
  component: NotificationCenter,
  decorators: [withRouter("/chat")],
  args: { state: STATE },
  parameters: { layout: "centered" },
  tags: ["ai-generated"],
} satisfies Meta<typeof NotificationCenter>;

export default meta;
type Story = StoryObj<typeof meta>;

export const WithUnread: Story = {};
export const Empty: Story = { args: { state: EMPTY_STATE } };

export const Opened: Story = {
  // The panel renders through a portal to document.body, so query the owner doc.
  play: async ({ canvas, canvasElement, userEvent }) => {
    await userEvent.click(canvas.getByTestId("notification-bell"));
    const body = within(canvasElement.ownerDocument.body);
    await expect(await body.findByTestId("notification-panel")).toBeVisible();
  },
};
