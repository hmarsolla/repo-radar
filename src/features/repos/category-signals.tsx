import { useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";

import { commands, unwrap } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import type { RepoRecord } from "@/bindings";

const CATEGORIES = [
  "Frontend",
  "Backend",
  "Fullstack",
  "Mobile",
  "DevOps",
  "DataMl",
  "Library",
  "Cli",
  "Docs",
  "Unknown",
] as const;

type FiredRule = {
  ruleId: string;
  signal: string;
  category: string;
  weight: number;
};
type CategoryScores = {
  totals: [string, number][];
  fired: FiredRule[];
};

/**
 * CategorySignals (FR-3.6 / FR-3.7, DESIGN §14.3). Shows every rule that
 * fired, its signal, its weight, and the per-category totals — the mechanism
 * by which the classifier earns trust. The override control sits right next
 * to that evidence so correcting a wrong call is one click from the thing
 * that was wrong; the computed value stays visible beside the override.
 */
export function CategorySignals({ repo }: { repo: RepoRecord }) {
  const qc = useQueryClient();
  const scores = useMemo(() => parseScores(repo.categoryScores), [repo.categoryScores]);

  const computed = repo.category ?? "Unknown";
  const manual = repo.categoryManual;
  const effective = manual ?? computed;

  const [pending, setPending] = useState<string | null>(null);
  const override = useMutation({
    mutationFn: (category: string | null) =>
      unwrap(commands.setRepoCategory(repo.id, category)),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["repo", repo.id] }),
    onSettled: () => setPending(null),
  });

  const maxTotal = Math.max(1, ...scores.totals.map(([, w]) => w));

  return (
    <section className="rounded-lg border bg-card p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-sm font-semibold">Classification</h2>
        <div className="flex items-center gap-2">
          <span className="text-lg font-semibold">{effective}</span>
          {manual ? (
            <span className="rounded bg-secondary px-1.5 py-0.5 text-[11px] text-muted-foreground">
              manual · computed {computed}
            </span>
          ) : (
            <span
              className="rounded bg-secondary px-1.5 py-0.5 text-[11px] capitalize text-muted-foreground"
              title="Confidence from the margin between the top two categories"
            >
              {repo.categoryConfidence ?? "low"} confidence
            </span>
          )}
        </div>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-2 text-sm">
        <label htmlFor="cat-override" className="text-muted-foreground">
          Override
        </label>
        <select
          id="cat-override"
          className="rounded-md border bg-background px-2 py-1 text-sm"
          value={manual ?? ""}
          disabled={override.isPending}
          onChange={(e) => {
            const v = e.target.value || null;
            setPending(v ?? "__clear");
            override.mutate(v);
          }}
        >
          <option value="">— use computed ({computed})</option>
          {CATEGORIES.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
        {manual ? (
          <button
            type="button"
            className="text-xs text-muted-foreground underline hover:text-foreground disabled:opacity-50"
            disabled={override.isPending}
            onClick={() => {
              setPending("__clear");
              override.mutate(null);
            }}
          >
            clear
          </button>
        ) : null}
        {override.isPending ? (
          <span className="text-xs text-muted-foreground">saving {pending}…</span>
        ) : null}
        {override.isError ? (
          <span className="text-xs text-compromise">could not save override</span>
        ) : null}
      </div>

      {scores.totals.length === 0 ? (
        <p className="mt-4 text-sm text-muted-foreground">
          No rules fired — nothing in this repo pointed at a category, so it is
          classified <b>Unknown</b> rather than guessed.
        </p>
      ) : (
        <>
          <div className="mt-4">
            <h3 className="mb-1.5 text-xs font-medium text-muted-foreground">
              Per-category weight
            </h3>
            <ul className="space-y-1">
              {scores.totals.map(([cat, weight]) => (
                <li key={cat} className="text-sm">
                  <div className="flex justify-between">
                    <span className={cn(cat === effective && "font-medium")}>{cat}</span>
                    <span className="tabular-nums text-muted-foreground">
                      {weight.toFixed(0)}
                    </span>
                  </div>
                  <div className="mt-0.5 h-1 overflow-hidden rounded-full bg-secondary">
                    <div
                      className="h-full bg-primary"
                      style={{ width: `${(weight / maxTotal) * 100}%` }}
                    />
                  </div>
                </li>
              ))}
            </ul>
          </div>

          <div className="mt-4">
            <h3 className="mb-1.5 text-xs font-medium text-muted-foreground">
              Rules that fired
            </h3>
            <ul className="divide-y rounded-lg border">
              {scores.fired.map((f, i) => (
                <li
                  key={`${f.ruleId}-${f.category}-${i}`}
                  className="flex items-center justify-between gap-3 px-3 py-2 text-sm"
                >
                  <span className="min-w-0">
                    <span className="font-mono text-xs">{f.ruleId}</span>
                    <span className="ml-2 text-muted-foreground">{f.signal}</span>
                  </span>
                  <span className="shrink-0 tabular-nums text-xs text-muted-foreground">
                    +{f.weight.toFixed(0)} → {f.category}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        </>
      )}
    </section>
  );
}

function parseScores(json: string | null): CategoryScores {
  if (!json) return { totals: [], fired: [] };
  try {
    const parsed = JSON.parse(json) as Partial<CategoryScores>;
    return {
      totals: Array.isArray(parsed.totals) ? parsed.totals : [],
      fired: Array.isArray(parsed.fired) ? parsed.fired : [],
    };
  } catch {
    return { totals: [], fired: [] };
  }
}
