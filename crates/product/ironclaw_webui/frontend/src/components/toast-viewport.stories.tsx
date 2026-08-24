import type { Meta, StoryObj } from "@storybook/react-vite";
import toast from "react-hot-toast";
import { expect } from "storybook/test";

import { Button } from "../design-system/button";
import { ToastViewport } from "./toast-viewport";

/**
 * ToastViewport renders nothing until a toast is fired through the shared
 * react-hot-toast store, so the demo pairs it with trigger buttons.
 */
function ToastDemo() {
  return (
    <div className="flex flex-wrap gap-3">
      <Button variant="secondary" size="sm" onClick={() => toast.success("Saved changes")}>
        Success toast
      </Button>
      <Button variant="danger" size="sm" onClick={() => toast.error("Something went wrong")}>
        Error toast
      </Button>
      <Button variant="ghost" size="sm" onClick={() => toast("Heads up")}>
        Info toast
      </Button>
      <ToastViewport />
    </div>
  );
}

const meta = {
  title: "Components/ToastViewport",
  component: ToastDemo,
  tags: ["ai-generated"],
} satisfies Meta<typeof ToastDemo>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const FiresToast: Story = {
  play: async ({ canvas, userEvent }) => {
    await userEvent.click(canvas.getByRole("button", { name: /success toast/i }));
    await expect(await canvas.findByTestId("toast")).toBeVisible();
  },
};
