import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

import { ConfirmDialog } from "./confirm-dialog";

const meta = {
  title: "Primitives/ConfirmDialog",
  component: ConfirmDialog,
  args: {
    open: true,
    title: "Delete workspace?",
    confirmLabel: "Delete",
    onConfirm: fn(),
    onCancel: fn(),
  },
  tags: ["ai-generated"],
} satisfies Meta<typeof ConfirmDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  args: { description: "This permanently removes the workspace and everything in it." },
};

export const WithoutDescription: Story = {};

export const Confirming: Story = {
  args: { description: "Removing…", isConfirming: true },
};

export const CustomLabels: Story = {
  args: {
    title: "Sign out everywhere?",
    description: "You'll need to sign in again on all devices.",
    confirmLabel: "Sign out",
    cancelLabel: "Stay signed in",
  },
};
