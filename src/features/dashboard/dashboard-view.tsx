import { PageHeader } from "@/components/page-header";

/**
 * Dashboard (PRD §6): repo count, health distribution, category donut,
 * language bar, stalest / worst-health repos, compromise banner. Built in
 * **M3-6 / M3-7**.
 */
export function DashboardView() {
  return (
    <div>
      <PageHeader
        title="Dashboard"
        description="Fleet overview — health distribution, categories, and the repos that need attention."
      />
      <PlaceholderGrid />
    </div>
  );
}

function PlaceholderGrid() {
  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {[
        "Repository count",
        "Health distribution",
        "Categories",
        "Languages",
        "Stalest repositories",
        "Worst health",
      ].map((label) => (
        <div
          key={label}
          className="flex h-40 flex-col rounded-lg border bg-card p-4"
        >
          <span className="text-sm font-medium text-muted-foreground">
            {label}
          </span>
          <span className="m-auto text-xs text-muted-foreground/60">
            populated after first scan
          </span>
        </div>
      ))}
    </div>
  );
}
