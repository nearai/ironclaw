import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from "../../src/extras/command";
import { Button } from "../../src/components/button";
import { Icon } from "../../src/primitives/icon";

const meta: Meta = { title: "Extras/Command" };
export default meta;

type Story = StoryObj;

function Palette() {
  return (
    <Command className="w-96">
      <CommandInput placeholder="Type a command or search…" />
      <CommandList>
        <CommandEmpty>No results found.</CommandEmpty>
        <CommandGroup heading="Runs">
          <CommandItem value="New run" keywords="create start">
            <Icon name="play" className="h-3.5 w-3.5 text-[var(--v2-text-faint)]" />
            New run
            <CommandShortcut>⌘N</CommandShortcut>
          </CommandItem>
          <CommandItem value="Pause all runs" keywords="stop halt">
            <Icon name="pause" className="h-3.5 w-3.5 text-[var(--v2-text-faint)]" />
            Pause all runs
          </CommandItem>
          <CommandItem value="Archive run" disabled>
            <Icon name="folder" className="h-3.5 w-3.5 text-[var(--v2-text-faint)]" />
            Archive run
          </CommandItem>
        </CommandGroup>
        <CommandGroup heading="Settings">
          <CommandItem value="Open settings" keywords="preferences config">
            <Icon name="settings" className="h-3.5 w-3.5 text-[var(--v2-text-faint)]" />
            Open settings
            <CommandShortcut>⌘,</CommandShortcut>
          </CommandItem>
          <CommandItem value="Toggle theme" keywords="dark light appearance">
            <Icon name="moon" className="h-3.5 w-3.5 text-[var(--v2-text-faint)]" />
            Toggle theme
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </Command>
  );
}

export const Inline: Story = { render: () => <Palette /> };

function DialogDemo() {
  const [open, setOpen] = React.useState(false);
  return (
    <>
      <Button variant="secondary" onClick={() => setOpen(true)}>
        Open command palette
      </Button>
      <CommandDialog open={open} onClose={() => setOpen(false)}>
        <Palette />
      </CommandDialog>
    </>
  );
}

export const Dialog: Story = { render: () => <DialogDemo /> };
