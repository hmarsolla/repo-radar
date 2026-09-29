import {
  AlertTriangle,
  Archive,
  CloudOff,
  FileQuestion,
  FilePen,
  Inbox,
  ShieldCheck,
  UploadCloud,
} from "lucide-react";

import { cn } from "@/lib/utils";
import type { Activity, Risk, RiskKind } from "@/bindings";

/**
 * How each risk reads to the user. The wording is deliberately about
 * *consequence* ("only copy is this folder") rather than about git internals,
 * because the decision being made is "can I delete this?".
 */
const RISK_META: Record<
  RiskKind,
  { label: (n: number | null) => string; icon: typeof FilePen; title: string }
> = {
  uncommitted_changes: {
    icon: FilePen,
    label: (n) => (n ? `${n} uncommitted` : "uncommitted changes"),
    title: "Tracked files are modified or staged but not committed.",
  },
  untracked_files: {
    icon: FileQuestion,
    label: (n) => (n ? `${n} untracked` : "untracked files"),
    title: "Files exist that git is not tracking and .gitignore does not cover.",
  },
  unpushed_commits: {
    icon: UploadCloud,
    label: (n) => (n ? `${n} unpushed` : "unpushed commits"),
    title: "Commits exist locally that the remote does not have.",
  },
  stash: {
    icon: Inbox,
    label: () => "stashed work",
    title: "This repository has at least one stash entry.",
  },
  no_remote: {
    icon: CloudOff,
    label: () => "no remote",
    title: "No remote is configured — this folder is the only copy.",
  },
};

export function RiskBadges({
  risks,
  className,
}: {
  risks: Risk[];
  className?: string;
}) {
  if (risks.length === 0) {
    return (
      <span
        className={cn(
          "inline-flex items-center gap-1 rounded-full border border-ok/40 bg-ok/10 px-2 py-0.5 text-xs text-ok",
          className,
        )}
        title="Clean working tree, everything pushed, and a remote exists."
      >
        <ShieldCheck className="size-3" />
        safe
      </span>
    );
  }

  return (
    <span className={cn("inline-flex flex-wrap items-center gap-1", className)}>
      {risks.map((risk) => {
        const meta = RISK_META[risk.kind];
        const Icon = meta.icon;
        return (
          <span
            key={risk.kind}
            title={meta.title}
            className="inline-flex items-center gap-1 rounded-full border border-warn/40 bg-warn/10 px-2 py-0.5 text-xs text-warn"
          >
            <Icon className="size-3" />
            {meta.label(risk.count)}
          </span>
        );
      })}
    </span>
  );
}

const ACTIVITY_META: Record<
  Activity,
  { label: string; className: string; title: string }
> = {
  active: {
    label: "active",
    className: "border-ok/40 bg-ok/10 text-ok",
    title: "Committed to within the last 90 days.",
  },
  dormant: {
    label: "dormant",
    className: "border-border bg-secondary text-muted-foreground",
    title: "No commit in 90 days.",
  },
  stale: {
    label: "stale",
    className: "border-warn/40 bg-warn/10 text-warn",
    title: "No commit in over a year — a plausible archive candidate.",
  },
  abandoned: {
    label: "abandoned",
    className: "border-warn/50 bg-warn/15 text-warn",
    title: "No commit in over two years.",
  },
  unknown: {
    label: "no commits",
    className: "border-border bg-secondary text-muted-foreground",
    title: "No commit history — an unborn repository.",
  },
};

export function ActivityBadge({ activity }: { activity: Activity }) {
  const meta = ACTIVITY_META[activity];
  return (
    <span
      title={meta.title}
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs",
        meta.className,
      )}
    >
      {activity === "stale" || activity === "abandoned" ? (
        <Archive className="size-3" />
      ) : activity === "unknown" ? (
        <AlertTriangle className="size-3" />
      ) : null}
      {meta.label}
    </span>
  );
}
