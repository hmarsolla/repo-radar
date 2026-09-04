import { useMemo, useState } from "react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { Play, Search } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { commands, unwrap } from "@/lib/ipc";
import type { RepoFilter, RepoSort } from "@/bindings";
import { useScan } from "@/features/scan/scan-provider";
import { Onboarding, useHasScanRoot } from "@/features/onboarding/onboarding";
import { ScanWarnings } from "@/features/system/scan-warnings";
import { RepoListTable } from "./repo-list-table";

const SORTS: { value: RepoSort; label: string }[] = [
  { value: "last_commit", label: "Last commit" },
  { value: "name", label: "Name" },
  { value: "primary_language", label: "Language" },
];

export function ReposView() {
  const hasRoot = useHasScanRoot();
  const scan = useScan();

  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<RepoSort>("last_commit");
  const [dirtyOnly, setDirtyOnly] = useState(false);

  const filter: RepoFilter = useMemo(
    () => ({
      search: search.trim() || null,
      language: null,
      dirtyOnly,
      includeBare: true,
      sort,
      descending: sort !== "name",
    }),
    [search, sort, dirtyOnly],
  );

  const repos = useQuery({
    queryKey: ["repos", filter],
    queryFn: () => unwrap(commands.listRepos(filter)),
    placeholderData: keepPreviousData,
    enabled: hasRoot.data === true,
  });

  const lastScan = useQuery({
    queryKey: ["latestScan"],
    queryFn: () => unwrap(commands.latestScanSummary()),
    enabled: hasRoot.data === true,
  });

  if (hasRoot.isLoading) {
    return <p className="p-6 text-sm text-muted-foreground">Loading…</p>;
  }
  if (!hasRoot.data) {
    return <Onboarding />;
  }

  const list = repos.data ?? [];
  const listEmpty = list.length === 0 && !repos.isFetching;
  const scanned = lastScan.data;
  const warnings = scan.running
    ? scan.warnings
    : (scanned?.warnings ?? []);

  const emptyMessage = scan.running
    ? "Scanning… repositories will appear here as they complete."
    : scanned?.status === "complete"
      ? "Scan complete — no git repositories were found under your scan roots. Check the folders in Settings, or that they contain git repositories."
      : scanned?.status === "cancelled"
        ? "The last scan was cancelled before any repositories were recorded."
        : scanned?.status === "failed"
          ? "The last scan failed. See the message above, then try again."
          : 'No repositories scanned yet. Click "Scan now" to start.';

  return (
    <div>
      <PageHeader
        title="Repositories"
        description="Every repository found under your scan roots."
        actions={
          <Button onClick={() => scan.start()} disabled={scan.running}>
            <Play />
            {scan.running ? "Scanning…" : "Scan now"}
          </Button>
        }
      />

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
          onChange={(e) => setSort(e.target.value as RepoSort)}
          className="h-9 rounded-md border bg-background px-2 text-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          {SORTS.map((s) => (
            <option key={s.value} value={s.value}>
              Sort: {s.label}
            </option>
          ))}
        </select>
        <label className="flex items-center gap-1.5 text-sm text-muted-foreground">
          <input
            type="checkbox"
            checked={dirtyOnly}
            onChange={(e) => setDirtyOnly(e.target.checked)}
          />
          Dirty only
        </label>
        <span className="ml-auto text-sm tabular-nums text-muted-foreground">
          {list.length} {list.length === 1 ? "repo" : "repos"}
        </span>
      </div>

      {scan.error ? (
        <div className="mb-4 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          Scan failed: {scan.error}
        </div>
      ) : null}

      {warnings.length > 0 ? (
        <ScanWarnings warnings={warnings} className="mb-4" />
      ) : null}

      {listEmpty ? (
        <div className="rounded-lg border bg-card p-8 text-center text-sm text-muted-foreground">
          {emptyMessage}
        </div>
      ) : (
        <RepoListTable repos={list} />
      )}
    </div>
  );
}
