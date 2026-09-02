import { ShieldCheck } from "lucide-react";

import { cn } from "@/lib/utils";

type FreshnessState = "never" | "fresh" | "stale" | "very-stale";

const LABELS: Record<FreshnessState, string> = {
  never: "Advisories not synced",
  fresh: "Advisories up to date",
  stale: "Advisories >7 days old",
  "very-stale": "Advisories >30 days old",
};

/**
 * Persistent slot for advisory-database freshness, visible from every route
 * (DESIGN §14.3, FR-5.6). Escalates past 7 and 30 days; "never synced" is
 * its own state — health is *unknown*, not healthy (DESIGN §14.4). Wired to
 * `get_sync_status` in **M2-20**; until then it reflects the only real
 * state: nothing has been synced.
 */
export function FreshnessIndicator({
  state = "never",
}: {
  state?: FreshnessState;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 rounded-md px-2 py-1 text-xs",
        state === "never" && "text-unknown",
        state === "fresh" && "text-ok",
        state === "stale" && "text-warn",
        state === "very-stale" && "text-vulnerability",
      )}
      title={LABELS[state]}
    >
      <ShieldCheck className="size-3.5" />
      <span className="hidden sm:inline">{LABELS[state]}</span>
    </div>
  );
}
