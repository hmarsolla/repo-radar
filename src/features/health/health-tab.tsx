import { AlertOctagon, ShieldAlert, TriangleAlert } from "lucide-react";

import { cn } from "@/lib/utils";
import type { FindingDetail, RepoDetail } from "@/bindings";

type Deduction = {
  cause: { Advisory: string } | "NoLockfile" | "StaleCommits" | "DirtyTree";
  label: string;
  amount: number;
  multipliers: [string, number][];
};

const BAND_TONE: Record<string, string> = {
  unknown: "text-unknown",
  critical: "text-compromise",
  poor: "text-vulnerability",
  fair: "text-warn",
  good: "text-ok",
  excellent: "text-ok",
};

/**
 * Health tab (FR-6.9, M2-19). Renders the stored `health_breakdown` JSON
 * **directly** — the number shown and the number explained cannot drift.
 * Compromise findings render first and visually distinct from
 * vulnerabilities (FR-6.1).
 */
export function HealthTab({ detail }: { detail: RepoDetail }) {
  const band = detail.repo.healthBand ?? "unknown";
  const score = detail.repo.healthScore;
  const breakdown = parseBreakdown(detail.healthBreakdown);

  const compromises = detail.findings.filter((f) => f.kind === "compromise");
  const vulns = detail.findings.filter((f) => f.kind !== "compromise");

  if (band === "unknown") {
    return (
      <div className="rounded-lg border border-unknown/40 bg-unknown/5 p-6">
        <p className="font-medium text-unknown">Health is unknown</p>
        <p className="mt-1 text-sm text-muted-foreground">
          The advisory database has not been synced, so this repository has not
          been checked against known vulnerabilities or compromises. This is
          <b> not</b> a clean bill of health. Sync from the Advisories screen.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <div className="flex items-baseline gap-3">
        <span className={cn("text-5xl font-semibold tabular-nums", BAND_TONE[band])}>
          {score ?? "—"}
        </span>
        <span className={cn("text-lg capitalize", BAND_TONE[band])}>{band}</span>
      </div>

      {compromises.length > 0 ? (
        <section className="rounded-lg border border-compromise/50 bg-compromise/5 p-4">
          <h3 className="flex items-center gap-2 font-semibold text-compromise">
            <AlertOctagon className="size-4" />
            Confirmed compromise — {compromises.length}
          </h3>
          <p className="mt-1 text-xs text-muted-foreground">
            A backdoored or malicious package. This caps the score regardless of
            everything else.
          </p>
          <ul className="mt-2 space-y-2">
            {compromises.map((f) => (
              <FindingRow key={f.advisoryId} f={f} tone="compromise" />
            ))}
          </ul>
        </section>
      ) : null}

      {vulns.length > 0 ? (
        <section>
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <ShieldAlert className="size-4 text-vulnerability" />
            Vulnerabilities — {vulns.length}
          </h3>
          <ul className="mt-2 space-y-2">
            {vulns.map((f) => (
              <FindingRow key={f.advisoryId} f={f} tone="vulnerability" />
            ))}
          </ul>
        </section>
      ) : null}

      <section>
        <h3 className="mb-2 text-sm font-semibold">Score breakdown</h3>
        {breakdown.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No deductions — {score === 100 ? "a perfect score." : "score is at its floor."}
          </p>
        ) : (
          <ul className="divide-y rounded-lg border">
            {breakdown.map((d, i) => (
              <li key={i} className="flex items-center justify-between gap-3 px-3 py-2 text-sm">
                <span className="flex items-center gap-2">
                  <CauseIcon cause={d.cause} />
                  <span>{d.label}</span>
                  {d.multipliers.map(([name, v]) => (
                    <span
                      key={name}
                      className="rounded bg-secondary px-1 text-[11px] text-muted-foreground"
                      title={`${name}: ×${v}`}
                    >
                      ×{v}
                    </span>
                  ))}
                </span>
                <span className="tabular-nums text-vulnerability">
                  −{d.amount.toFixed(1)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

function FindingRow({
  f,
  tone,
}: {
  f: FindingDetail;
  tone: "compromise" | "vulnerability";
}) {
  return (
    <li className="rounded-md border bg-card p-2 text-sm">
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono text-xs">{f.advisoryId}</span>
        <span
          className={cn(
            "rounded px-1.5 py-0.5 text-[11px] uppercase",
            tone === "compromise" ? "bg-compromise/15 text-compromise" : "bg-vulnerability/15 text-vulnerability",
          )}
        >
          {f.severity} · {f.confidence}
        </span>
      </div>
      <div className="mt-1 text-muted-foreground">
        {f.packageName}@{f.packageVersion}
        {f.fixedVersion ? <> → fixed in {f.fixedVersion}</> : null}
      </div>
      {f.summary ? <div className="mt-1 text-xs">{f.summary}</div> : null}
    </li>
  );
}

function CauseIcon({ cause }: { cause: Deduction["cause"] }) {
  if (typeof cause === "object" && "Advisory" in cause) {
    return <ShieldAlert className="size-3.5 text-vulnerability" />;
  }
  return <TriangleAlert className="size-3.5 text-warn" />;
}

function parseBreakdown(json: string | null): Deduction[] {
  if (!json) return [];
  try {
    const parsed = JSON.parse(json);
    return Array.isArray(parsed) ? (parsed as Deduction[]) : [];
  } catch {
    return [];
  }
}
