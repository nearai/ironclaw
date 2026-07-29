import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { DataTable, Table, TableBody, TableCell, TableHead, TableHeader, TableRow, type DataTableColumn } from "./table";

type Run = { id: string; agent: string; tokens: number };

const COLUMNS: DataTableColumn<Run>[] = [
  { key: "id", header: "Run" },
  { key: "agent", header: "Agent" },
  { key: "tokens", header: "Tokens", align: "right", cell: (run) => run.tokens.toLocaleString("en-US") },
];

const ROWS: Run[] = [
  { id: "run-1", agent: "coder", tokens: 1200 },
  { id: "run-2", agent: "reviewer", tokens: 800 },
];

test("Table primitives render semantic table markup", () => {
  const html = renderToStaticMarkup(
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Name</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow>
          <TableCell>Ada</TableCell>
        </TableRow>
      </TableBody>
    </Table>
  );
  assert.match(html, /<table[^>]*>/);
  assert.match(html, /<th[^>]*>Name<\/th>/);
  assert.match(html, /<td[^>]*>Ada<\/td>/);
});

test("DataTable renders columns, custom cells, and row order", () => {
  const html = renderToStaticMarkup(
    <DataTable columns={COLUMNS} rows={ROWS} rowKey={(run) => run.id} />
  );
  assert.match(html, /Run.*Agent.*Tokens/s);
  assert.match(html, /run-1.*coder.*1,200/s);
  assert.match(html, /run-2.*reviewer.*800/s);
  assert.match(html, /text-right/);
});

test("DataTable shows the empty state across all columns", () => {
  const html = renderToStaticMarkup(
    <DataTable columns={COLUMNS} rows={[]} emptyState="No runs yet" />
  );
  assert.match(html, /colspan="3"/i);
  assert.match(html, /No runs yet/);
});
