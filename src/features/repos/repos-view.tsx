import { PageHeader } from "@/components/page-header";

/**
 * Repository list (DESIGN §14.3): TanStack Table + virtual rows, SQL-side
 * filtering and sorting. Built in **M1-10**; health / findings columns in
 * **M2-23**.
 */
export function ReposView() {
  return (
    <div>
      <PageHeader
        title="Repositories"
        description="Every repository found under your scan roots."
      />
      <div className="rounded-lg border bg-card p-8 text-center text-sm text-muted-foreground">
        No scan has run yet. Add a scan root in Settings, then start a scan.
      </div>
    </div>
  );
}
