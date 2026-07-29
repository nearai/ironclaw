/**
 * Table primitives + DataTable
 *
 * Semantic table elements styled with the v2 tokens (no tanstack dep):
 *   Table / TableHeader / TableBody / TableFooter / TableRow / TableHead /
 *   TableCell / TableCaption — thin wrappers over the native elements.
 *
 * DataTable<Row> renders rows from a plain column definition:
 *   columns: { key, header, cell?(row), className?, align? }[]
 *   rows:    Row[]  (keyed via rowKey, defaulting to the array index)
 *
 * Usage
 *   <DataTable
 *     columns={[
 *       { key: "name", header: "Name" },
 *       { key: "status", header: "Status", cell: (r) => <Badge>{r.status}</Badge> },
 *     ]}
 *     rows={rows}
 *     rowKey={(r) => r.id}
 *   />
 */
import type { ComponentProps, ReactNode } from "react";
import { cn } from "../primitives/cn";

/* ── Primitives ────────────────────────────────────────────────────── */

export function Table({ className, ...props }: ComponentProps<"table">) {
  return (
    <div className="relative w-full overflow-x-auto">
      <table
        className={cn("w-full caption-bottom border-collapse text-ui", className)}
        {...props}
      />
    </div>
  );
}

export function TableHeader({ className, ...props }: ComponentProps<"thead">) {
  return (
    <thead
      className={cn("border-b border-[var(--v2-panel-border)]", className)}
      {...props}
    />
  );
}

export function TableBody({ className, ...props }: ComponentProps<"tbody">) {
  return <tbody className={className} {...props} />;
}

export function TableFooter({ className, ...props }: ComponentProps<"tfoot">) {
  return (
    <tfoot
      className={cn(
        "border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] font-medium",
        className
      )}
      {...props}
    />
  );
}

export function TableRow({ className, ...props }: ComponentProps<"tr">) {
  return (
    <tr
      className={cn(
        "border-b border-[var(--v2-panel-border)] transition-colors last:border-b-0",
        "hover:bg-[var(--v2-surface-soft)] data-[state=selected]:bg-[var(--v2-accent-soft)]",
        className
      )}
      {...props}
    />
  );
}

export function TableHead({ className, ...props }: ComponentProps<"th">) {
  return (
    <th
      className={cn(
        "h-10 px-3 text-left align-middle text-ui-sm font-medium text-[var(--v2-text-faint)]",
        className
      )}
      {...props}
    />
  );
}

export function TableCell({ className, ...props }: ComponentProps<"td">) {
  return (
    <td
      className={cn("px-3 py-2.5 align-middle text-[var(--v2-text)]", className)}
      {...props}
    />
  );
}

export function TableCaption({ className, ...props }: ComponentProps<"caption">) {
  return (
    <caption
      className={cn("mt-3 text-ui-sm text-[var(--v2-text-faint)]", className)}
      {...props}
    />
  );
}

/* ── DataTable ─────────────────────────────────────────────────────── */

export type DataTableColumn<Row> = {
  /** Unique column id; doubles as the row property to render when no cell(). */
  key: string;
  header: ReactNode;
  /** Custom cell renderer; defaults to String(row[key]). */
  cell?: (row: Row) => ReactNode;
  align?: "left" | "right" | "center";
  className?: string;
};

type DataTableProps<Row> = {
  columns: DataTableColumn<Row>[];
  rows: Row[];
  /** Stable row key; defaults to the row index. */
  rowKey?: (row: Row, index: number) => string | number;
  /** Rendered inside a full-width row when rows is empty. */
  emptyState?: ReactNode;
  caption?: ReactNode;
  className?: string;
};

const ALIGN_CLASSES = {
  left: "text-left",
  right: "text-right",
  center: "text-center",
};

function defaultCellValue<Row>(row: Row, key: string): ReactNode {
  const value = (row as Record<string, unknown>)[key];
  if (value == null) return null;
  if (typeof value === "string" || typeof value === "number") return value;
  return String(value);
}

export function DataTable<Row>({
  columns,
  rows,
  rowKey,
  emptyState,
  caption,
  className,
}: DataTableProps<Row>) {
  return (
    <Table className={className}>
      {caption ? <TableCaption>{caption}</TableCaption> : null}
      <TableHeader>
        <TableRow className="hover:bg-transparent">
          {columns.map((column) => (
            <TableHead
              key={column.key}
              className={cn(ALIGN_CLASSES[column.align ?? "left"], column.className)}
            >
              {column.header}
            </TableHead>
          ))}
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.length === 0 ? (
          <TableRow>
            <TableCell
              colSpan={columns.length}
              className="py-8 text-center text-[var(--v2-text-faint)]"
            >
              {emptyState ?? "No results"}
            </TableCell>
          </TableRow>
        ) : (
          rows.map((row, index) => (
            <TableRow key={rowKey ? rowKey(row, index) : index}>
              {columns.map((column) => (
                <TableCell
                  key={column.key}
                  className={cn(ALIGN_CLASSES[column.align ?? "left"], column.className)}
                >
                  {column.cell ? column.cell(row) : defaultCellValue(row, column.key)}
                </TableCell>
              ))}
            </TableRow>
          ))
        )}
      </TableBody>
    </Table>
  );
}
