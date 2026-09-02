import { PageHeader } from "@/components/page-header";

/**
 * Advisories screen (DESIGN §14.3, FR-5.6): sync status and history,
 * per-ecosystem counts, **Sync now**, and the cross-repo impact view.
 * Built in **M2-20**.
 */
export function AdvisoriesView() {
  return (
    <div>
      <PageHeader
        title="Advisories"
        description="OSV advisory database status and cross-repository impact."
      />
      <div className="rounded-lg border bg-card p-8 text-center text-sm text-muted-foreground">
        The advisory database has never been synced.
      </div>
    </div>
  );
}
