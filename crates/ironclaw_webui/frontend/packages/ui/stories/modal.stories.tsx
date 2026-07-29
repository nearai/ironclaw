import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Modal, ModalBody, ModalFooter } from "../src/components/modal";
import { Button } from "../src/components/button";

function ModalDemo({ size }: { size?: "sm" | "md" | "lg" | "xl" | "full" }) {
  const [open, setOpen] = React.useState(false);
  return (
    <>
      <Button variant="secondary" size="sm" onClick={() => setOpen(true)}>
        Open modal
      </Button>
      <Modal open={open} onClose={() => setOpen(false)} title="Configure extension" size={size}>
        <ModalBody>
          <p className="text-sm text-[var(--v2-text-muted)]">
            Backdrop click and Escape both close. Body scroll locks while open.
          </p>
        </ModalBody>
        <ModalFooter>
          <Button variant="secondary" size="sm" onClick={() => setOpen(false)}>Cancel</Button>
          <Button size="sm" onClick={() => setOpen(false)}>Save</Button>
        </ModalFooter>
      </Modal>
    </>
  );
}

const meta: Meta<typeof Modal> = {
  title: "Components/Modal",
  component: Modal,
};
export default meta;

type Story = StoryObj<typeof Modal>;

export const Default: Story = { render: () => <ModalDemo /> };
export const Large: Story = { render: () => <ModalDemo size="lg" /> };
