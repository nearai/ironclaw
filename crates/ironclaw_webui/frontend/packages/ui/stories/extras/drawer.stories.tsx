import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Drawer, DrawerBody, DrawerFooter, type DrawerSide } from "../../src/extras/drawer";
import { Button } from "../../src/components/button";

const meta: Meta = { title: "Extras/Drawer" };
export default meta;

type Story = StoryObj;

function Demo({ side }: { side: DrawerSide }) {
  const [open, setOpen] = React.useState(false);
  return (
    <>
      <Button variant="secondary" onClick={() => setOpen(true)}>
        Open {side} drawer
      </Button>
      <Drawer open={open} onClose={() => setOpen(false)} side={side} title="Run details">
        <DrawerBody>
          <p>
            Edge-anchored panel for secondary flows: inspection, filters,
            quick edits. Escape, backdrop click, or the close button dismiss it.
          </p>
        </DrawerBody>
        <DrawerFooter>
          <Button variant="ghost" onClick={() => setOpen(false)}>Cancel</Button>
          <Button onClick={() => setOpen(false)}>Save</Button>
        </DrawerFooter>
      </Drawer>
    </>
  );
}

export const Right: Story = { render: () => <Demo side="right" /> };
export const Left: Story = { render: () => <Demo side="left" /> };
export const Bottom: Story = { render: () => <Demo side="bottom" /> };
