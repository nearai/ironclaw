import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";
import { MemoryRouter, Outlet, Route, Routes } from "react-router";

import { withQueryClient } from "../test-support/storybook-decorators";
import { GatewayLayout } from "./gateway-layout";

// The full app shell: sidebar + header + routed <Outlet>. Seeding the ["threads"]
// and ["trace-credits"] queries renders it without a backend; isAdmin={false}
// skips the LLM-providers fetch and the first-run onboarding redirect.
const THREADS_DATA = {
  threads: [
    { thread_id: "t1", title: "Deploy pipeline audit", updated_at: "2026-07-30T14:00:00Z", state: null },
    { thread_id: "t2", title: "Q3 roadmap notes", updated_at: "2026-07-29T09:30:00Z", state: null },
  ],
  next_cursor: null,
};

const PROFILE = { id: "u_ada", display_name: "Ada Lovelace", email: "ada@ironclaw.dev", role: "member" };

function OutletContent() {
  // Placeholder for the routed page that GatewayLayout renders into its Outlet.
  return (
    <div className="grid h-full place-items-center text-sm text-[var(--v2-text-muted)]">
      Routed page content (Outlet)
      <Outlet />
    </div>
  );
}

const meta = {
  title: "Components/GatewayLayout",
  component: GatewayLayout,
  decorators: [
    withQueryClient((client) => {
      client.setQueryData(["threads"], THREADS_DATA);
      client.setQueryData(["trace-credits"], { enrolled: false });
    }),
  ],
  parameters: { layout: "fullscreen" },
  args: {
    token: "demo-token",
    profile: PROFILE,
    isAdmin: false,
    isChecking: false,
    rebornProjectsEnabled: false,
    onSignOut: fn(),
  },
  render: (args) => (
    <MemoryRouter initialEntries={["/chat"]}>
      <Routes>
        <Route element={<GatewayLayout {...args} />}>
          <Route path="chat" element={<OutletContent />} />
        </Route>
      </Routes>
    </MemoryRouter>
  ),
  tags: ["ai-generated"],
} satisfies Meta<typeof GatewayLayout>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
