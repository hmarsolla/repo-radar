import { useMemo, useRef } from "react";
import { useNavigate } from "react-router-dom";
import {
  flexRender,
  getCoreRowModel,
  useReactTable,
  type ColumnDef,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { GitBranch, CircleDot } from "lucide-react";

import type { RepoListItem } from "@/bindings";
import { cn } from "@/lib/utils";

function relativeTime(iso: string | null): string {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "—";
  const diff = Date.now() - then;
  const day = 86_400_000;
  if (diff < day) return "today";
  if (diff < 2 * day) return "yesterday";
  if (diff < 30 * day) return `${Math.round(diff / day)}d ago`;
  if (diff < 365 * day) return `${Math.round(diff / (30 * day))}mo ago`;
  return `${Math.round(diff / (365 * day))}y ago`;
}

const columns: ColumnDef<RepoListItem>[] = [
  {
    accessorKey: "name",
    header: "Repository",
    cell: ({ row }) => (
      <div className="flex flex-col">
        <span className="font-medium">{row.original.name}</span>
        <span className="truncate text-xs text-muted-foreground" title={row.original.path}>
          {row.original.path}
        </span>
      </div>
    ),
  },
  {
    accessorKey: "primaryLanguage",
    header: "Language",
    cell: ({ getValue }) => (
      <span className="text-sm text-muted-foreground">
        {(getValue() as string | null) ?? "—"}
      </span>
    ),
  },
  {
    accessorKey: "branch",
    header: "Branch",
    cell: ({ getValue, row }) =>
      row.original.isBare ? (
        <span className="text-xs text-muted-foreground">bare</span>
      ) : (
        <span className="inline-flex items-center gap-1 text-sm text-muted-foreground">
          <GitBranch className="size-3.5" />
          {(getValue() as string | null) ?? "detached"}
        </span>
      ),
  },
  {
    accessorKey: "lastCommitAt",
    header: "Last commit",
    cell: ({ getValue }) => (
      <span className="text-sm text-muted-foreground">
        {relativeTime(getValue() as string | null)}
      </span>
    ),
  },
  {
    accessorKey: "dirty",
    header: "",
    cell: ({ getValue }) =>
      (getValue() as boolean) ? (
        <span
          className="inline-flex items-center gap-1 text-xs text-warn"
          title="Working tree has uncommitted changes"
        >
          <CircleDot className="size-3.5" />
          dirty
        </span>
      ) : null,
  },
];

export function RepoListTable({ repos }: { repos: RepoListItem[] }) {
  const navigate = useNavigate();
  const parentRef = useRef<HTMLDivElement>(null);

  const table = useReactTable({
    data: repos,
    columns,
    getCoreRowModel: getCoreRowModel(),
  });

  const rows = table.getRowModel().rows;
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 56,
    overscan: 12,
  });

  const gridCols = useMemo(
    () => "minmax(16rem,1fr) 8rem 10rem 8rem 5rem",
    [],
  );

  return (
    <div className="rounded-lg border bg-card">
      <div
        className="grid items-center gap-4 border-b px-4 py-2 text-xs font-medium uppercase tracking-wide text-muted-foreground"
        style={{ gridTemplateColumns: gridCols }}
      >
        {table.getHeaderGroups()[0].headers.map((header) => (
          <div key={header.id}>
            {flexRender(header.column.columnDef.header, header.getContext())}
          </div>
        ))}
      </div>

      <div ref={parentRef} className="max-h-[calc(100vh-18rem)] overflow-auto">
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((vi) => {
            const row = rows[vi.index];
            return (
              <div
                key={row.id}
                className={cn(
                  "absolute left-0 top-0 grid w-full cursor-pointer items-center gap-4 border-b px-4 text-sm hover:bg-accent/50",
                )}
                style={{
                  height: vi.size,
                  transform: `translateY(${vi.start}px)`,
                  gridTemplateColumns: gridCols,
                }}
                onClick={() => navigate(`/repos/${row.original.id}`)}
              >
                {row.getVisibleCells().map((cell) => (
                  <div key={cell.id} className="min-w-0">
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
