import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "../../src/extras/collapsible";
import { Button } from "../../src/components/button";

const meta: Meta = { title: "Extras/Collapsible" };
export default meta;

type Story = StoryObj;

function Demo() {
  const [open, setOpen] = React.useState(false);
  return (
    <Collapsible open={open} onOpenChange={setOpen} className="w-80">
      <div className="flex items-center justify-between gap-3">
        <span className="text-ui font-medium text-[var(--v2-text-strong)]">
          3 hidden runs
        </span>
        <CollapsibleTrigger asChild>
          <Button variant="ghost" size="sm">{open ? "Hide" : "Show"}</Button>
        </CollapsibleTrigger>
      </div>
      <CollapsibleContent className="mt-2 flex flex-col gap-1.5">
        {["run-4821", "run-4820", "run-4819"].map((run) => (
          <div
            key={run}
            className="rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2 text-ui text-[var(--v2-text)]"
          >
            {run}
          </div>
        ))}
      </CollapsibleContent>
    </Collapsible>
  );
}

export const Default: Story = { render: () => <Demo /> };
