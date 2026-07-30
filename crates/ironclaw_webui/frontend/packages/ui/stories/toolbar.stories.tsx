import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Toolbar, ToolbarGroup } from "../src/composites/toolbar";
import { SearchInput } from "../src/components/search-input";
import { Select } from "../src/components/input";
import { Button } from "../src/components/button";
import { Icon } from "../src/icons/icon";

const meta: Meta = { title: "Composites/Toolbar" };
export default meta;

type Story = StoryObj;

function Demo() {
  const [query, setQuery] = React.useState("");
  return (
    <div className="w-[44rem]">
      <Toolbar>
        <SearchInput
          label="Search jobs"
          placeholder="Search job title or UUID"
          value={query}
          onChange={(event) => setQuery(event.currentTarget.value)}
          onClear={() => setQuery("")}
          clearLabel="Clear search"
          className="md:flex-1"
        />
        <ToolbarGroup>
          <Select size="sm" aria-label="State filter" className="w-40">
            <option>All states</option>
            <option>Running</option>
            <option>Failed</option>
          </Select>
          <Button variant="secondary" size="icon-sm" aria-label="Refresh">
            <Icon name="retry" className="h-4 w-4" />
          </Button>
        </ToolbarGroup>
      </Toolbar>
    </div>
  );
}

export const Default: Story = { render: () => <Demo /> };
