import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { SearchInput } from "../src/components/search-input";

const meta: Meta = { title: "Components/SearchInput" };
export default meta;

type Story = StoryObj;

function Demo() {
  const [query, setQuery] = React.useState("nightly");
  return (
    <div className="w-96">
      <SearchInput
        label="Search settings"
        placeholder="Search settings..."
        value={query}
        onChange={(event) => setQuery(event.currentTarget.value)}
        onClear={() => setQuery("")}
        clearLabel="Clear search"
      />
    </div>
  );
}

export const Default: Story = { render: () => <Demo /> };

export const Sizes: Story = {
  render: () => (
    <div className="grid w-96 gap-3">
      <SearchInput label="Small" placeholder="Small (toolbar default)" size="sm" />
      <SearchInput label="Medium" placeholder="Medium" size="md" />
    </div>
  ),
};
