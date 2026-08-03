import type { Meta, StoryObj } from "@storybook/react-vite";

import { withQueryClient, withRouter } from "../test-support/storybook-decorators";
import { SidebarTraceCredits } from "./sidebar-trace-credits";

// SidebarTraceCredits reads the shared ["trace-credits"] query and renders only
// when enrolled. Seeding the cache renders its loaded state with no network.
const ENROLLED = {
  enrolled: true,
  final_credit: 12.5,
  submissions_accepted: 8,
  submissions_submitted: 10,
  manual_review_hold_count: 0,
};

const seed = (credits: unknown) =>
  withQueryClient((client) => client.setQueryData(["trace-credits"], credits));

const meta = {
  title: "Components/SidebarTraceCredits",
  component: SidebarTraceCredits,
  decorators: [withRouter("/chat")],
  parameters: { layout: "centered" },
  tags: ["ai-generated"],
} satisfies Meta<typeof SidebarTraceCredits>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Enrolled: Story = {
  decorators: [seed(ENROLLED)],
};

export const WithHold: Story = {
  decorators: [seed({ ...ENROLLED, manual_review_hold_count: 2 })],
};

// Not enrolled → the component renders nothing (kept for documentation).
export const NotEnrolled: Story = {
  decorators: [seed({ enrolled: false })],
};
