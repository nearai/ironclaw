import type { Meta, StoryObj } from "@storybook/react-vite";
import { Popover, PopoverContent, PopoverTrigger } from "../../src/popover";
import { Button } from "../../src/button";
import { FormField, Input } from "../../src/input";

const meta = {
  title: "Components/Popover",
  component: Popover,
  parameters: {
    docs: {
      description: {
        component:
          "Anchored surface on `@radix-ui/react-popover` for small inline forms and " +
          "detail panes. Focus moves into the panel on open and returns to the " +
          "trigger on close; Escape and outside-click dismiss.",
      },
    },
  },
} satisfies Meta<typeof Popover>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="secondary" size="sm">Rename thread</Button>
      </PopoverTrigger>
      <PopoverContent className="w-72" align="start">
        <FormField label="Thread name">
          <Input defaultValue="Inbox triage — Monday" />
        </FormField>
        <div className="mt-3 flex justify-end">
          <Button size="sm">Save</Button>
        </div>
      </PopoverContent>
    </Popover>
  ),
};
