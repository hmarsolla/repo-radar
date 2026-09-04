import { useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { AlertTriangle, ArrowRight, Loader2, RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { commands, unwrap, IpcError } from "@/lib/ipc";
import type { OutdatedEntry, OutdatedReport, OutdatedStatus } from "@/bindings";

/**
 * Updates tab (FR-8, M5-2). The check runs **only** on an explicit click,
 * and the panel says up front that it contacts external package registries
 * (FR-8.7). Outdated-ness is a maintenance fact, not a security one — it
 * never touches the health score (FR-8.6), and this UI states so.
 */
export function OutdatedTab({ repoId }: { repoId: number }) {
  const [showCurrent, setShowCurrent] = useState(false);

  const check = useMutation({
    mutationFn: (forceRefresh: boolean) =>
      unwrap(commands.checkOutdated(repoId, forceRefresh)),
  });

  const report = check.data as OutdatedReport | undefined;

  if (!report) {
    return (
      <div className="max-w-xl rounded-lg border bg-card p-5">
        <h3 className="text-sm font-semibold">Check for updates</h3>
        <p className="mt-1 text-sm text-muted-foreground">
          Looks up the latest published version of every dependency in this
          repository. This <b>contacts external package registries</b> — the npm
          registry, PyPI, crates.io, and the Go module proxy — sending each
          package name (never anything about your code). Results are cached for
          24&nbsp;hours.
        </p>
        <p className="mt-2 text-sm text-muted-foreground">
          Being behind on versions does <b>not</b> affect the health score.
        </p>
        {check.isError ? (
          <p className="mt-3 text-sm text-vulnerability">
            {check.error instanceof IpcError
              ? check.error.message
              : "Check failed."}
          </p>
        ) : null}
        <Button
          className="mt-4"
          onClick={() => check.mutate(false)}
          disabled={check.isPending}
        >
          {check.isPending ? (
            <>
              <Loader2 className="animate-spin" />
              Contacting registries…
            </>
          ) : (
            <>
              <RefreshCw />
              Check for updates
            </>
          )}
        </Button>
      </div>
    );
  }

  return (
    <Result
      report={report}
      pending={check.isPending}
      error={check.isError ? check.error : null}
      showCurrent={showCurrent}
      onToggleCurrent={() => setShowCurrent((v) => !v)}
      onRecheck={() => check.mutate(true)}
    />
  );
}

function Result({
  report,
  pending,
  error,
  showCurrent,
  onToggleCurrent,
  onRecheck,
}: {
  report: OutdatedReport;
  pending: boolean;
  error: unknown;
  showCurrent: boolean;
  onToggleCurrent: () => void;
  onRecheck: () => void;
}) {
  const behind = useMemo(
    () => report.entries.filter((e) => isBehind(e.status)),
    [report.entries],
  );
  const current = report.entries.length - behind.length;
  const shown = showCurrent ? report.entries : behind;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="text-sm">
          <span className="font-medium">
            {behind.length === 0
              ? "Everything is up to date"
              : `${behind.length} of ${report.entries.length} dependencies behind`}
          </span>
          <span className="ml-2 text-muted-foreground">
            checked {ago(report.generatedAt)}
          </span>
        </div>
        <Button
          size="sm"
          variant="outline"
          onClick={onRecheck}
          disabled={pending}
        >
          {pending ? <Loader2 className="animate-spin" /> : <RefreshCw />}
          Re-check now
        </Button>
      </div>

      <p className="text-xs text-muted-foreground">
        Version lag does not affect the health score — that signal is reserved
        for known vulnerabilities and compromises.
      </p>

      {error ? (
        <p className="text-sm text-vulnerability">
          {error instanceof IpcError ? error.message : "Re-check failed."}
        </p>
      ) : null}

      {report.failed.length > 0 ? (
        <div className="rounded-lg border border-warn/40 bg-warn/5 p-3 text-sm">
          <p className="flex items-center gap-2 font-medium text-warn">
            <AlertTriangle className="size-4" />
            {report.failed.length} lookup
            {report.failed.length === 1 ? "" : "s"} failed
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            These packages could not be checked, so this list is incomplete —
            not a clean bill of health. Try Re-check now.
          </p>
          <ul className="mt-1.5 flex flex-wrap gap-1.5">
            {report.failed.map((f) => (
              <li
                key={f}
                className="rounded bg-secondary px-1.5 py-0.5 font-mono text-[11px]"
              >
                {f}
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {report.entries.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No dependencies were parsed for this repository.
        </p>
      ) : (
        <>
          <ul className="divide-y rounded-lg border">
            {shown.map((e) => (
              <EntryRow key={rowKey(e)} e={e} />
            ))}
            {shown.length === 0 ? (
              <li className="px-3 py-6 text-center text-sm text-muted-foreground">
                Nothing behind.
              </li>
            ) : null}
          </ul>
          {current > 0 ? (
            <button
              onClick={onToggleCurrent}
              className="text-xs text-muted-foreground underline underline-offset-2 hover:text-foreground"
            >
              {showCurrent
                ? `Hide ${current} up-to-date`
                : `Show ${current} up-to-date`}
            </button>
          ) : null}
        </>
      )}
    </div>
  );
}

function EntryRow({ e }: { e: OutdatedEntry }) {
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2 text-sm">
      <StatusBadge status={e.status} />
      <span className="font-mono text-xs">{e.rawName}</span>
      {!e.isDirect ? (
        <span className="rounded bg-secondary px-1 text-[10px] uppercase text-muted-foreground">
          transitive
        </span>
      ) : null}
      {e.scope !== "runtime" ? (
        <span className="rounded bg-secondary px-1 text-[10px] uppercase text-muted-foreground">
          {e.scope}
        </span>
      ) : null}

      <span className="ml-auto flex items-center gap-1.5 tabular-nums">
        <span className="text-muted-foreground">{e.currentVersion}</span>
        {e.latestVersion && e.latestVersion !== e.currentVersion ? (
          <>
            <ArrowRight className="size-3 text-muted-foreground" />
            <span className="font-medium">{e.latestVersion}</span>
          </>
        ) : e.latestVersion == null ? (
          <span
            className="text-muted-foreground"
            title={e.error ?? "Registry did not return a version"}
          >
            {e.error ? "lookup failed" : "unknown"}
          </span>
        ) : null}
      </span>

      <span className="w-full text-[11px] text-muted-foreground/70">
        {e.ecosystem}
        {" · "}
        {e.manifestPath}
        {e.fromCache ? ` · cached ${ago(e.checkedAt)}` : ""}
      </span>
    </li>
  );
}

const STATUS_LABEL: Record<OutdatedStatus, string> = {
  upToDate: "current",
  outdatedPatch: "patch",
  outdatedMinor: "minor",
  outdatedMajor: "major",
  unknown: "?",
};

const STATUS_TONE: Record<OutdatedStatus, string> = {
  upToDate: "bg-ok/10 text-ok",
  outdatedPatch: "bg-secondary text-muted-foreground",
  outdatedMinor: "bg-warn/10 text-warn",
  outdatedMajor: "bg-warn/20 text-warn",
  unknown: "bg-unknown/10 text-unknown",
};

function StatusBadge({ status }: { status: OutdatedStatus }) {
  return (
    <span
      className={cn(
        "w-14 shrink-0 rounded px-1.5 py-0.5 text-center text-[11px] font-medium uppercase",
        STATUS_TONE[status],
      )}
    >
      {STATUS_LABEL[status]}
    </span>
  );
}

function isBehind(s: OutdatedStatus): boolean {
  return s === "outdatedPatch" || s === "outdatedMinor" || s === "outdatedMajor";
}

function rowKey(e: OutdatedEntry): string {
  return `${e.ecosystem}:${e.name}:${e.currentVersion}:${e.manifestPath}`;
}

function ago(iso: string | null | undefined): string {
  if (!iso) return "never";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "recently";
  const mins = (Date.now() - then) / 60_000;
  if (mins < 1) return "just now";
  if (mins < 60) return `${Math.round(mins)}m ago`;
  const h = mins / 60;
  if (h < 24) return `${Math.round(h)}h ago`;
  return `${Math.round(h / 24)}d ago`;
}
