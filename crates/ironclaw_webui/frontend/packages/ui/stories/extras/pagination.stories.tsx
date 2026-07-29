import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Pagination,
  PaginationButton,
  PaginationContent,
  PaginationEllipsis,
  PaginationItem,
  PaginationNext,
  PaginationPrevious,
  SimplePagination,
} from "../../src/extras/pagination";

const meta: Meta = { title: "Extras/Pagination" };
export default meta;

type Story = StoryObj;

export const Composed: Story = {
  render: () => (
    <Pagination>
      <PaginationContent>
        <PaginationItem><PaginationPrevious disabled /></PaginationItem>
        <PaginationItem><PaginationButton isActive>1</PaginationButton></PaginationItem>
        <PaginationItem><PaginationButton>2</PaginationButton></PaginationItem>
        <PaginationItem><PaginationButton>3</PaginationButton></PaginationItem>
        <PaginationItem><PaginationEllipsis /></PaginationItem>
        <PaginationItem><PaginationButton>12</PaginationButton></PaginationItem>
        <PaginationItem><PaginationNext /></PaginationItem>
      </PaginationContent>
    </Pagination>
  ),
};

function SimpleDemo() {
  const [page, setPage] = React.useState(7);
  return <SimplePagination page={page} pageCount={20} onPageChange={setPage} />;
}

export const Simple: Story = { render: () => <SimpleDemo /> };

function FewPagesDemo() {
  const [page, setPage] = React.useState(1);
  return <SimplePagination page={page} pageCount={4} onPageChange={setPage} />;
}

export const FewPages: Story = { render: () => <FewPagesDemo /> };
