import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, within } from "storybook/test";
import { MemoryRouter, Outlet, Route, Routes } from "react-router";

import { withQueryClient, withStubbedFetch } from "../test-support/storybook-decorators";
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

const PROFILE = { tenant_id: "tenant-demo", user_id: "u_ada" };

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

export const PaletteFocusRestoration: Story = {
  args: { token: "" },
  decorators: [withStubbedFetch([])],
  render: (args) => (
    <MemoryRouter initialEntries={["/chat"]}>
      <Routes>
        <Route element={<GatewayLayout {...args} />}>
          <Route path="chat" element={
            <label>
              Draft
              <textarea aria-label="Draft" />
            </label>
          } />
        </Route>
      </Routes>
    </MemoryRouter>
  ),
  play: async ({ canvas, canvasElement, userEvent }) => {
    const draft = canvas.getByRole("textbox", { name: "Draft" });
    await userEvent.type(draft, "Keep this draft");

    for (const dismissal of ["Escape", "Control", "Meta", "backdrop"]) {
      const focused = new Promise<void>((resolve) => {
        canvasElement.addEventListener("focusin", function onFocus(event) {
          if (event.target instanceof HTMLElement && event.target.closest('[role="dialog"]')) {
            canvasElement.removeEventListener("focusin", onFocus);
            resolve();
          }
        });
      });
      await userEvent.keyboard("{Control>}k{/Control}");
      await focused;
      const dialog = await canvas.findByRole("dialog");
      await expect(within(dialog).getByRole("textbox")).toHaveFocus();

      if (dismissal === "backdrop") {
        await userEvent.click(within(dialog).getByRole("button", { name: "Close" }));
      } else if (dismissal === "Escape") {
        await userEvent.keyboard("{Escape}");
      } else {
        await userEvent.keyboard(`{${dismissal}>}k{/${dismissal}}`);
      }

      await expect(canvas.queryByRole("dialog")).not.toBeInTheDocument();
      await expect(draft).toHaveFocus();
    }

    await userEvent.keyboard(" preserved");
    await expect(draft).toHaveValue("Keep this draft preserved");
  },
};
