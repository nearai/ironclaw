import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Modal, ModalBody, ModalFooter, ModalHeader } from "../../src/modal";
import { ConfirmDialog } from "../../src/confirm-dialog";
import { Button } from "../../src/button";
import { FormField, Input } from "../../src/input";

const meta = {
  title: "Components/Modal",
  component: Modal,
  parameters: {
    docs: {
      description: {
        component:
          "Dialog on `@radix-ui/react-dialog`: focus trap, Escape/scrim dismissal, " +
          "aria-modal, and focus return to the trigger — all inherited from the " +
          "primitive. The panel enters on the restrained modal motion (scrim fade + " +
          "scale from center, quicker exit) and sits on `--v2-shadow-modal`. " +
          "`ConfirmDialog` is the packaged destructive-action pattern.",
      },
      // The Modal deliberately omits a portal (SSR consumers), so its
      // fixed overlay is clipped by the inline docs canvas. Render docs
      // stories in a real iframe viewport instead.
      story: { inline: false, iframeHeight: 460 },
    },
  },
  argTypes: {
    size: { control: "select", options: ["sm", "md", "lg", "xl", "full"] },
  },
  args: { size: "md" },
} satisfies Meta<typeof Modal>;

export default meta;
type Story = StoryObj;

function ModalDemo({ size }: { size?: "sm" | "md" | "lg" | "xl" | "full" }) {
  const [open, setOpen] = useState(true);
  return (
    <>
      <Button variant="secondary" onClick={() => setOpen(true)}>
        Configure extension
      </Button>
      <Modal open={open} onClose={() => setOpen(false)} size={size}>
        <ModalHeader onClose={() => setOpen(false)}>Configure extension</ModalHeader>
        <ModalBody>
          <FormField label="Webhook URL" hint="Events are POSTed here as JSON.">
            <Input placeholder="https://example.com/hooks/ironclaw" />
          </FormField>
        </ModalBody>
        <ModalFooter>
          <Button variant="secondary" size="sm" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button size="sm" onClick={() => setOpen(false)}>
            Save
          </Button>
        </ModalFooter>
      </Modal>
    </>
  );
}

export const Default: Story = {
  render: (args: { size?: "sm" | "md" | "lg" | "xl" | "full" }) => (
    <ModalDemo size={args.size} />
  ),
};

export const Confirm: Story = {
  render: function ConfirmStory() {
    const [open, setOpen] = useState(true);
    return (
      <>
        <Button variant="danger" onClick={() => setOpen(true)}>
          Delete automation
        </Button>
        <ConfirmDialog
          open={open}
          title="Delete this automation?"
          description="“Morning digest” will stop running immediately. This cannot be undone."
          confirmLabel="Delete"
          onConfirm={() => setOpen(false)}
          onCancel={() => setOpen(false)}
        />
      </>
    );
  },
};
