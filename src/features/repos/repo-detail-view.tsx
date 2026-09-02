import { useParams } from "react-router-dom";

import { PageHeader } from "@/components/page-header";

/**
 * Repo detail: overview, Health tab (DESIGN §14.3, M2-19), dependencies,
 * category evidence (M3-4), prompt generation (M4). Built incrementally.
 */
export function RepoDetailView() {
  const { id } = useParams();
  return (
    <div>
      <PageHeader
        title={`Repository #${id ?? "?"}`}
        description="Health, dependencies, classification, and prompt tools for this repository."
      />
      <div className="rounded-lg border bg-card p-8 text-center text-sm text-muted-foreground">
        Detail view lands with the analysis pipeline (M1–M2).
      </div>
    </div>
  );
}
