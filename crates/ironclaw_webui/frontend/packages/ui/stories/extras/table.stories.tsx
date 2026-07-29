import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  DataTable,
  Table,
  TableBody,
  TableCaption,
  TableCell,
  TableFooter,
  TableHead,
  TableHeader,
  TableRow,
  type DataTableColumn,
} from "../../src/extras/table";
import { Badge } from "../../src/components/badge";

const meta: Meta = { title: "Extras/Table" };
export default meta;

type Story = StoryObj;

type Run = { id: string; agent: string; status: string; tokens: number };

const RUNS: Run[] = [
  { id: "run-4821", agent: "researcher", status: "running", tokens: 48210 },
  { id: "run-4820", agent: "coder", status: "done", tokens: 122400 },
  { id: "run-4819", agent: "reviewer", status: "failed", tokens: 8113 },
  { id: "run-4818", agent: "coder", status: "done", tokens: 66021 },
];

export const Primitives: Story = {
  render: () => (
    <div className="w-[30rem]">
      <Table>
        <TableCaption>Recent agent runs.</TableCaption>
        <TableHeader>
          <TableRow>
            <TableHead>Run</TableHead>
            <TableHead>Agent</TableHead>
            <TableHead className="text-right">Tokens</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {RUNS.map((run) => (
            <TableRow key={run.id}>
              <TableCell className="font-medium text-[var(--v2-text-strong)]">{run.id}</TableCell>
              <TableCell>{run.agent}</TableCell>
              <TableCell className="text-right">{run.tokens.toLocaleString()}</TableCell>
            </TableRow>
          ))}
        </TableBody>
        <TableFooter>
          <TableRow>
            <TableCell colSpan={2}>Total</TableCell>
            <TableCell className="text-right">
              {RUNS.reduce((sum, run) => sum + run.tokens, 0).toLocaleString()}
            </TableCell>
          </TableRow>
        </TableFooter>
      </Table>
    </div>
  ),
};

const COLUMNS: DataTableColumn<Run>[] = [
  { key: "id", header: "Run" },
  { key: "agent", header: "Agent" },
  {
    key: "status",
    header: "Status",
    cell: (run) => (
      <Badge
        tone={run.status === "done" ? "positive" : run.status === "failed" ? "danger" : "info"}
        label={run.status}
      />
    ),
  },
  {
    key: "tokens",
    header: "Tokens",
    align: "right",
    cell: (run) => run.tokens.toLocaleString(),
  },
];

export const DataTableStory: Story = {
  name: "DataTable",
  render: () => (
    <div className="w-[32rem]">
      <DataTable columns={COLUMNS} rows={RUNS} rowKey={(run) => run.id} />
    </div>
  ),
};

export const DataTableEmpty: Story = {
  render: () => (
    <div className="w-[32rem]">
      <DataTable columns={COLUMNS} rows={[]} emptyState="No runs yet" />
    </div>
  ),
};
