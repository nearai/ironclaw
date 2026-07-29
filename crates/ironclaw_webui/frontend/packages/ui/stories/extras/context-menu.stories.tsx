import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  ContextMenu,
  ContextMenuCheckboxItem,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuRadioGroup,
  ContextMenuRadioItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "../../src/extras/context-menu";

const meta: Meta = { title: "Extras/ContextMenu" };
export default meta;

type Story = StoryObj;

function Demo() {
  const [pinned, setPinned] = React.useState(true);
  const [sort, setSort] = React.useState("recent");
  return (
    <ContextMenu>
      <ContextMenuTrigger
        className="grid h-36 w-72 place-items-center rounded-[12px] border border-dashed border-[var(--v2-panel-border)] text-ui text-[var(--v2-text-muted)]"
      >
        Right-click here
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuLabel>Run actions</ContextMenuLabel>
        <ContextMenuItem>
          Rename<ContextMenuShortcut>⌘R</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuCheckboxItem
          checked={pinned}
          onCheckedChange={(next) => setPinned(next === true)}
        >
          Pinned
        </ContextMenuCheckboxItem>
        <ContextMenuSub>
          <ContextMenuSubTrigger>Sort by</ContextMenuSubTrigger>
          <ContextMenuSubContent>
            <ContextMenuRadioGroup value={sort} onValueChange={setSort}>
              <ContextMenuRadioItem value="recent">Most recent</ContextMenuRadioItem>
              <ContextMenuRadioItem value="name">Name</ContextMenuRadioItem>
            </ContextMenuRadioGroup>
          </ContextMenuSubContent>
        </ContextMenuSub>
        <ContextMenuSeparator />
        <ContextMenuItem tone="danger">Delete run</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

export const Default: Story = { render: () => <Demo /> };
