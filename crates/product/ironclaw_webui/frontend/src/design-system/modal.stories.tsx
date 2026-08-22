import type { Meta, StoryObj } from "@storybook/react-vite";
import { useEffect, useState } from "react";

import { Button } from "./button";
import { Modal, ModalBody, ModalFooter } from "./modal";

type ModalDemoProps = {
  open?: boolean;
  size?: "sm" | "md" | "lg" | "xl" | "full";
  withFooter?: boolean;
};

/** Modal requires `open`; this demo owns the state and a trigger button. */
function ModalDemo({ open: initialOpen = false, size = "md", withFooter = true }: ModalDemoProps) {
  const [open, setOpen] = useState(initialOpen);
  // Re-sync when the `open` control changes in the Storybook toolbar; otherwise
  // the demo would only ever reflect the initial arg from first mount.
  useEffect(() => setOpen(initialOpen), [initialOpen]);
  return (
    <>
      <Button onClick={() => setOpen(true)}>Open modal</Button>
      <Modal
        open={open}
        onClose={() => setOpen(false)}
        title="Delete workspace"
        size={size}
        closeLabel="Close"
      >
        <ModalBody>
          <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
            This permanently removes the workspace and everything in it. This action cannot be
            undone.
          </p>
        </ModalBody>
        {withFooter ? (
          <ModalFooter>
            <Button variant="secondary" size="sm" onClick={() => setOpen(false)}>Cancel</Button>
            <Button variant="danger" size="sm" onClick={() => setOpen(false)}>Delete</Button>
          </ModalFooter>
        ) : null}
      </Modal>
    </>
  );
}

const meta = {
  title: "Primitives/Modal",
  component: ModalDemo,
  args: { open: false, size: "md", withFooter: true },
  tags: ["ai-generated"],
} satisfies Meta<typeof ModalDemo>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Closed: Story = {};
export const Open: Story = { args: { open: true } };
export const Small: Story = { args: { open: true, size: "sm" } };
export const Large: Story = { args: { open: true, size: "lg" } };
export const WithoutFooter: Story = { args: { open: true, withFooter: false } };
