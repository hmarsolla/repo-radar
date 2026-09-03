import { useQuery } from "@tanstack/react-query";
import { Play } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { commands, unwrap } from "@/lib/ipc";
import { useScan } from "@/features/scan/scan-provider";
import { Onboarding, useHasScanRoot } from "@/features/onboarding/onboarding";

/**
 * Dashboard (PRD §6). Charts — health distribution, category donut, language
 * bar, compromise banner — arrive in **M3-6 / M3-7**. For M1 it shows the
 * repo count and a scan control.
 */
export function DashboardView() {
  const hasRoot = useHasScanRoot();
  const scan = useScan();

  const repos = useQuery({
    queryKey: ["repos", "dashboard-count"],
    queryFn: () => unwrap(commands.listRepos({})),
    enabled: hasRoot.data === true,
  });

  if (hasRoot.isLoading) {
    return <p className="p-6 text-sm text-muted-foreground">Loading…</p>;
  }
  if (!hasRoot.data) {
    return <Onboarding />;
  }

  const count = repos.data?.length ?? 0;
  const dirty = repos.data?.filter((r) => r.dirty).length ?? 0;

  return (
    <div>
      <PageHeader
        title="Dashboard"
        description="Fleet overview — health distribution, categories, and the repos that need attention."
        actions={
          <Button onClick={() => scan.start()} disabled={scan.running}>
            <Play />
            {scan.running ? "Scanning…" : "Scan now"}
          </Button>
        }
      />

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        <Stat label="Repositories" value={count} />
        <Stat label="Dirty working trees" value={dirty} />
        <Placeholder label="Health distribution" />
        <Placeholder label="Categories" />
        <Placeholder label="Languages" />
        <Placeholder label="Worst health" />
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex h-40 flex-col rounded-lg border bg-card p-4">
      <span className="text-sm font-medium text-muted-foreground">{label}</span>
      <span className="m-auto text-5xl font-semibold tabular-nums">{value}</span>
    </div>
  );
}

function Placeholder({ label }: { label: string }) {
  return (
    <div className="flex h-40 flex-col rounded-lg border bg-card p-4">
      <span className="text-sm font-medium text-muted-foreground">{label}</span>
      <span className="m-auto text-xs text-muted-foreground/60">
        arrives with classification & health (M2–M3)
      </span>
    </div>
  );
}
