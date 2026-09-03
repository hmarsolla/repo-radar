import { ShieldAlert, ShieldCheck, ShieldQuestion } from "lucide-react";
import { Link } from "react-router-dom";

import { cn } from "@/lib/utils";
import { useSyncStatus } from "@/features/advisories/use-sync";

/**
 * Advisory-database freshness, visible from every route (DESIGN §14.3,
 * FR-5.6). Escalates past 7 and 30 days; "never synced" is its own state —
 * health is *unknown*, not healthy (DESIGN §14.4, M2-22).
 */
export function FreshnessIndicator() {
  const status = useSyncStatus();
  const freshness = status.data?.freshness ?? "never";

  const meta = {
    never: { label: "Advisories not synced", cls: "text-unknown", Icon: ShieldQuestion },
    fresh: { label: "Advisories up to date", cls: "text-ok", Icon: ShieldCheck },
    stale: { label: "Advisories >7 days old", cls: "text-warn", Icon: ShieldAlert },
    "very-stale": {
      label: "Advisories >30 days old",
      cls: "text-vulnerability",
      Icon: ShieldAlert,
    },
  }[normalize(freshness)];

  return (
    <Link
      to="/advisories"
      className={cn(
        "flex items-center gap-2 rounded-md px-2 py-1 text-xs hover:bg-accent",
        meta.cls,
      )}
      title={meta.label}
    >
      <meta.Icon className="size-3.5" />
      <span className="hidden sm:inline">{meta.label}</span>
    </Link>
  );
}

function normalize(f: string): "never" | "fresh" | "stale" | "very-stale" {
  switch (f) {
    case "Never":
    case "never":
      return "never";
    case "Fresh":
    case "fresh":
      return "fresh";
    case "Stale":
    case "stale":
      return "stale";
    default:
      return "very-stale";
  }
}
