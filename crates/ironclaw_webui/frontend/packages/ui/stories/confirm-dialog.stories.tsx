import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { ConfirmDialog } from "../src/composites/confirm-dialog";
import { Button } from "../src/components/button";

const meta: Meta = { title: "Composites/ConfirmDialog" };
export default meta;

type Story = StoryObj;

function Demo() {
  const [open, setOpen] = React.useState(false);
  return (
    <>
      <Button variant="danger" size="sm" onClick={() => setOpen(true)}>Delete chat</Button>
      <ConfirmDialog
        open={open}
        title="Delete chat"
        description="This permanently removes the conversation."
        confirmLabel="Delete"
        onConfirm={() => setOpen(false)}
        onCancel={() => setOpen(false)}
      />
    </>
  );
}

export const Default: Story = { render: () => <Demo /> };
