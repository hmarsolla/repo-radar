import { useState } from "react";
import { ChevronDown, ChevronRight, TriangleAlert } from "lucide-react";

import { cn } from "@/lib/utils";
import { scopePath } from "@/lib/warnings";
import type { Warning } from "@/bindings";

const KIND_LABEL: Record<string, string> = {
  PermissionDenied: "permission denied",
  ParseFailed: "parse failed",
  GitTimeout: "git timed out",
  GitError: "git error",
  UnparseableVersion: "unparseable version",
  RulePackInvalid: "rule pack invalid",
  Panic: "internal panic (contained)",
  Other: "warning",
};

/**
 * The recoverable-problems surface (FR-1.10, DESIGN §14.4, §15). A scan
 * never fails as a whole: unreadable dirs, unparseable lockfiles, git
 * timeouts, and contained panics all become warnings. This makes them
 * non-silent — a repo that hit one must not look identical to a clean one.
 */
export function ScanWarnings({
  warnings,
  title = "Warnings from the last scan",
  className,
}: {
  warnings: Warning[];
  title?: string;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  if (warnings.length === 0) return null;

  return (
    <div
      className={cn(
        "rounded-lg border border-warn/40 bg-warn/5 text-sm",
        className,
      )}
    >
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 font-medium text-warn"
      >
        {open ? (
          <ChevronDown className="size-4" />
        ) : (
          <ChevronRight className="size-4" />
        )}
        <TriangleAlert className="size-4" />
        {warnings.length} {title}
      </button>
      {open ? (
        <ul className="divide-y divide-warn/20 border-t border-warn/20">
          {warnings.map((w, i) => {
            const path = scopePath(w);
            return (
              <li key={i} className="px-3 py-2">
                <span className="rounded bg-warn/15 px-1 text-[11px] uppercase text-warn">
                  {KIND_LABEL[w.kind] ?? w.kind}
                </span>{" "}
                <span className="text-foreground">{w.message}</span>
                {path ? (
                  <span className="ml-1 block truncate font-mono text-[11px] text-muted-foreground">
                    {path}
                  </span>
                ) : null}
              </li>
            );
          })}
        </ul>
      ) : null}
    </div>
  );
}
