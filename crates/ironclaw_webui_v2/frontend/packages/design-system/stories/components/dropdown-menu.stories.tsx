import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "../../src/dropdown-menu";
import { Button } from "../../src/button";
import { Icon } from "../../src/icons";

const meta = {
  title: "Components/DropdownMenu",
  component: DropdownMenu,
  parameters: {
    docs: {
      description: {
        component:
          "Action/command menu on `@radix-ui/react-dropdown-menu`: roving focus, " +
          "typeahead, submenu support, and dismissal handled by the primitive. " +
          "Sits on `--v2-shadow-menu` with the menu-entrance motion. For picking a " +
          "value, use SelectMenu instead.",
      },
    },
  },
} satisfies Meta<typeof DropdownMenu>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="secondary" size="sm">
          Actions
          <Icon name="chevron" className="ml-1.5 h-3.5 w-3.5 rotate-90" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        <DropdownMenuLabel>Morning digest</DropdownMenuLabel>
        <DropdownMenuItem>
          <Icon name="play" className="mr-2 h-4 w-4" />
          Run now
        </DropdownMenuItem>
        <DropdownMenuItem>
          <Icon name="edit" className="mr-2 h-4 w-4" />
          Edit schedule
        </DropdownMenuItem>
        <DropdownMenuItem>
          <Icon name="copy" className="mr-2 h-4 w-4" />
          Duplicate
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem className="text-[var(--v2-danger-text)] data-[highlighted]:text-[var(--v2-danger-text)]">
          <Icon name="trash" className="mr-2 h-4 w-4" />
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  ),
};
