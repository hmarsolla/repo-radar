import { useMemo, useState } from "react";
import { keepPreviousData, useMutation, useQuery } from "@tanstack/react-query";
import {
  ChevronDown,
  ChevronRight,
  FolderOpen,
  HardDrive,
  Info,
  Play,
  Search,
} from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { commands, unwrap } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { formatAge, formatBytes, formatCount } from "@/lib/format";
import type { CleanupFilter, CleanupRow, CleanupSort } from "@/bindings";
import { useScan } from "@/features/scan/scan-provider";
import { Onboarding, useHasScanRoot } from "@/features/onboarding/onboarding";
import { ActivityBadge, RiskBadges } from "./risk-badges";

const SORTS: { value: CleanupSort; label: string }[] = [
  { value: "reclaimable", label: "Reclaimable space" },
  { value: "totalSize", label: "Total size" },
  { value: "oldest", label: "Oldest commit" },
  { value: "name", label: "Name" },
];

export function CleanupView() {
  const hasRoot = useHasScanRoot();
  const scan = useScan();

  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<CleanupSort>("reclaimable");
  const [safeOnly, setSafeOnly] = useState(false);
  const [staleOnly, setStaleOnly] = useState(false);

  const filter: CleanupFilter = useMemo(
    () => ({ sort, safeOnly, staleOnly }),
    [sort, safeOnly, staleOnly],
  );

  const rows = useQuery({
    queryKey: ["cleanup", filter],
    queryFn: () => unwrap(commands.cleanupList(filter)),
    placeholderData: keepPreviousData,
    enabled: hasRoot.data === true,
  });

  const summary = useQuery({
    queryKey: ["cleanupSummary"],
    queryFn: () => unwrap(commands.cleanupSummary()),
    enabled: hasRoot.data === true,
  });

  if (hasRoot.isLoading) {
    return <p className="p-6 text-sm text-muted-foreground">Loading…</p>;
  }
  if (!hasRoot.data) {
    return <Onboarding />;
  }

  // Name/path filtering is client-side: the list is one row per repository, so
  // it is small, and filtering here keeps typing instant.
  const needle = search.trim().toLowerCase();
  const list = (rows.data ?? []).filter(
    (r) =>
      !needle ||
      r.name.toLowerCase().includes(needle) ||
      r.path.toLowerCase().includes(needle),
  );

  const s = summary.data;
  const neverMeasured = s ? s.repoCount - s.measuredCount : 0;

  return (
    <div>
      <PageHeader
        title="Cleanup"
        description="What each repository costs on disk, how much of that a build tool can regenerate, and which repositories hold work that exists nowhere else."
        actions={
          <Button onClick={() => scan.start()} disabled={scan.running}>
            <Play />
            {scan.running ? "Scanning…" : "Rescan"}
          </Button>
        }
      />

      {s ? (
        <div className="mb-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <Tile
            label="Total on disk"
            value={formatBytes(s.totalBytes)}
            hint={`across ${formatCount(s.measuredCount)} measured ${
              s.measuredCount === 1 ? "repository" : "repositories"
            }`}
          />
          <Tile
            label="Regenerable"
            value={formatBytes(s.reclaimableBytes)}
            hint="build output a tool rebuilds on demand"
          />
          <Tile
            label="Safe to clear"
            value={formatBytes(s.safeReclaimableBytes)}
            hint="in repositories with nothing at risk"
            tone="ok"
          />
          <Tile
            label="Holds unique work"
            value={formatCount(s.atRiskCount)}
            hint={`${formatCount(s.staleCount)} stale or abandoned`}
            tone={s.atRiskCount > 0 ? "warn" : undefined}
          />
        </div>
      ) : null}

      {s?.anyTruncated ? (
        <p className="mb-4 flex items-start gap-2 rounded-md border border-border bg-secondary px-3 py-2 text-xs text-muted-foreground">
          <Info className="mt-0.5 size-3.5 shrink-0" />
          At least one repository hit the per-repo file limit while being
          measured, so the sizes above are lower bounds.
        </p>
      ) : null}

      {neverMeasured > 0 ? (
        <p className="mb-4 flex items-start gap-2 rounded-md border border-border bg-secondary px-3 py-2 text-xs text-muted-foreground">
          <Info className="mt-0.5 size-3.5 shrink-0" />
          {formatCount(neverMeasured)}{" "}
          {neverMeasured === 1 ? "repository has" : "repositories have"} never
          been measured. Rescan to fill in their disk usage.
        </p>
      ) : null}

      <div className="mb-4 flex flex-wrap items-center gap-2">
        <div className="relative">
          <Search className="pointer-events-none absolute left-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Filter by name or path"
            className="h-9 w-64 rounded-md border bg-background pl-8 pr-3 text-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
          />
        </div>
        <select
          value={sort}
          onChange={(e) => setSort(e.target.value as CleanupSort)}
          className="h-9 rounded-md border bg-background px-2 text-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          {SORTS.map((o) => (
            <option key={o.value} value={o.value}>
              Sort: {o.label}
            </option>
          ))}
        </select>
        <label className="flex items-center gap-1.5 text-sm text-muted-foreground">
          <input
            type="checkbox"
            checked={safeOnly}
            onChange={(e) => setSafeOnly(e.target.checked)}
          />
          Safe only
        </label>
        <label className="flex items-center gap-1.5 text-sm text-muted-foreground">
          <input
            type="checkbox"
            checked={staleOnly}
            onChange={(e) => setStaleOnly(e.target.checked)}
          />
          Stale only
        </label>
        <span className="ml-auto text-sm tabular-nums text-muted-foreground">
          {list.length} {list.length === 1 ? "repo" : "repos"}
        </span>
      </div>

      {list.length === 0 ? (
        <div className="rounded-lg border bg-card p-8 text-center text-sm text-muted-foreground">
          {rows.isFetching
            ? "Loading…"
            : safeOnly || staleOnly || needle
              ? "No repositories match these filters."
              : "No repositories scanned yet. Run a scan to measure disk usage."}
        </div>
      ) : (
        <div className="overflow-hidden rounded-lg border bg-card">
          <table className="w-full text-sm">
            <thead className="border-b bg-secondary/50 text-left text-xs uppercase tracking-wide text-muted-foreground">
              <tr>
                <th className="w-8" />
                <th className="px-3 py-2 font-medium">Repository</th>
                <th className="px-3 py-2 font-medium">Last commit</th>
                <th className="px-3 py-2 text-right font-medium">Total</th>
                <th className="px-3 py-2 text-right font-medium">
                  Regenerable
                </th>
                <th className="px-3 py-2 font-medium">Risk</th>
                <th className="w-10" />
              </tr>
            </thead>
            <tbody>
              {list.map((row) => (
                <CleanupTableRow key={row.repoId} row={row} />
              ))}
            </tbody>
          </table>
        </div>
      )}

      <p className="mt-4 text-xs text-muted-foreground">
        Repo Radar never deletes anything. Open a folder to clear it with your
        own file manager — regenerable directories come back the next time you
        run <code>npm install</code>, <code>cargo build</code>, or your
        project’s equivalent.
      </p>
    </div>
  );
}

function CleanupTableRow({ row }: { row: CleanupRow }) {
  const [open, setOpen] = useState(false);
  const reveal = useMutation({
    mutationFn: (relPath: string | null) =>
      unwrap(commands.revealPath(row.repoId, relPath)),
  });

  const expandable = row.reclaimableDirs.length > 0;
  const reclaimShare =
    row.totalBytes && row.totalBytes > 0 && row.reclaimableBytes !== null
      ? row.reclaimableBytes / row.totalBytes
      : 0;

  return (
    <>
      <tr className="border-b last:border-b-0 align-middle">
        <td className="pl-2">
          {expandable ? (
            <button
              onClick={() => setOpen((v) => !v)}
              className="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-accent"
              aria-label={open ? "Hide directories" : "Show directories"}
              aria-expanded={open}
            >
              {open ? (
                <ChevronDown className="size-4" />
              ) : (
                <ChevronRight className="size-4" />
              )}
            </button>
          ) : null}
        </td>

        <td className="px-3 py-2">
          <div className="font-medium">{row.name}</div>
          <div className="truncate text-xs text-muted-foreground" title={row.path}>
            {row.path}
          </div>
        </td>

        <td className="px-3 py-2">
          <div className="flex flex-col items-start gap-1">
            <span className="text-muted-foreground">
              {formatAge(row.daysSinceCommit)}
            </span>
            <ActivityBadge activity={row.activity} />
          </div>
        </td>

        <td className="px-3 py-2 text-right tabular-nums">
          {formatBytes(row.totalBytes)}
          {row.truncated ? (
            <span title="Measurement hit the file limit; this is a lower bound.">
              {" "}
              +
            </span>
          ) : null}
          {row.gitBytes ? (
            <div
              className="text-xs text-muted-foreground"
              title="Size of .git — history, not working files."
            >
              {formatBytes(row.gitBytes)} git
            </div>
          ) : null}
        </td>

        <td className="px-3 py-2 text-right tabular-nums">
          <span className={reclaimShare > 0.5 ? "font-medium text-warn" : ""}>
            {formatBytes(row.reclaimableBytes)}
          </span>
          {reclaimShare > 0 ? (
            <div className="text-xs text-muted-foreground">
              {Math.round(reclaimShare * 100)}% of total
            </div>
          ) : null}
        </td>

        <td className="px-3 py-2">
          <RiskBadges risks={row.risks} />
        </td>

        <td className="pr-2">
          <button
            onClick={() => reveal.mutate(null)}
            title="Open this repository in your file manager"
            aria-label={`Open ${row.name} in file manager`}
            className="flex size-7 items-center justify-center rounded text-muted-foreground hover:bg-accent"
          >
            <FolderOpen className="size-4" />
          </button>
        </td>
      </tr>

      {open ? (
        <tr className="border-b bg-secondary/30 last:border-b-0">
          <td />
          <td colSpan={6} className="px-3 py-2">
            <ul className="space-y-1">
              {row.reclaimableDirs.map((dir) => (
                <li
                  key={dir.relPath}
                  className="flex items-center gap-2 text-xs"
                >
                  <HardDrive className="size-3 shrink-0 text-muted-foreground" />
                  <code className="truncate">{dir.relPath}</code>
                  <span className="tabular-nums text-muted-foreground">
                    {formatBytes(dir.bytes)} · {formatCount(dir.fileCount)}{" "}
                    files
                  </span>
                  <button
                    onClick={() => reveal.mutate(dir.relPath)}
                    className="ml-auto shrink-0 text-muted-foreground underline hover:text-foreground"
                  >
                    open
                  </button>
                </li>
              ))}
            </ul>
            {reveal.isError ? (
              <p className="mt-2 text-xs text-destructive">
                Could not open that folder — it may already be deleted. Rescan to
                refresh.
              </p>
            ) : null}
          </td>
        </tr>
      ) : null}
    </>
  );
}

function Tile({
  label,
  value,
  hint,
  tone,
}: {
  label: string;
  value: string;
  hint?: string;
  tone?: "ok" | "warn";
}) {
  return (
    <div className="rounded-lg border bg-card p-3">
      <div className="text-xs uppercase tracking-wide text-muted-foreground">
        {label}
      </div>
      <div
        className={cn(
          "mt-1 text-2xl font-semibold tabular-nums",
          tone === "ok" && "text-ok",
          tone === "warn" && "text-warn",
        )}
      >
        {value}
      </div>
      {hint ? (
        <div className="mt-0.5 text-xs text-muted-foreground">{hint}</div>
      ) : null}
    </div>
  );
}
