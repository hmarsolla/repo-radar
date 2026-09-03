import { useMemo, useRef } from "react";
import { useNavigate } from "react-router-dom";
import {
  flexRender,
  getCoreRowModel,
  useReactTable,
  type ColumnDef,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { AlertOctagon, CircleDot } from "lucide-react";

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

const BAND_TONE: Record<string, string> = {
  unknown: "bg-unknown/15 text-unknown",
  critical: "bg-compromise/15 text-compromise",
  poor: "bg-vulnerability/15 text-vulnerability",
  fair: "bg-warn/15 text-warn",
  good: "bg-ok/15 text-ok",
  excellent: "bg-ok/15 text-ok",
};

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
    id: "health",
    header: "Health",
    cell: ({ row }) => {
      const band = row.original.healthBand ?? "unknown";
      const score = row.original.healthScore;
      return (
        <span
          className={cn(
            "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-xs capitalize",
            BAND_TONE[band],
          )}
          title={band === "unknown" ? "Not checked — advisories not synced" : undefined}
        >
          {band === "unknown" ? "unknown" : (score ?? "—")}
        </span>
      );
    },
  },
  {
    id: "compromise",
    header: "Compromise",
    cell: ({ row }) =>
      row.original.compromiseCount > 0 ? (
        <span
          className="inline-flex items-center gap-1 rounded bg-compromise/15 px-1.5 py-0.5 text-xs font-medium text-compromise"
          title="Confirmed malicious / backdoored packages"
        >
          <AlertOctagon className="size-3.5" />
          {row.original.compromiseCount}
        </span>
      ) : (
        <span className="text-xs text-muted-foreground">0</span>
      ),
  },
  {
    id: "vulnerability",
    header: "Vulns",
    cell: ({ row }) =>
      row.original.vulnerabilityCount > 0 ? (
        <span className="inline-flex items-center gap-1 rounded bg-vulnerability/15 px-1.5 py-0.5 text-xs font-medium text-vulnerability">
          {row.original.vulnerabilityCount}
        </span>
      ) : (
        <span className="text-xs text-muted-foreground">0</span>
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
    // name · health · compromise · vulns · language · last commit · dirty
    () => "minmax(14rem,1fr) 5rem 6rem 4rem 7rem 8rem 5rem",
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
