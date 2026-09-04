import { useState } from "react";
import { useParams, Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, GitBranch, GitCommitHorizontal, Users } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { cn } from "@/lib/utils";
import { commands, unwrap } from "@/lib/ipc";
import { HealthTab } from "@/features/health/health-tab";
import { CategorySignals } from "@/features/repos/category-signals";
import { OutdatedTab } from "@/features/repos/outdated-tab";
import { ScanWarnings } from "@/features/system/scan-warnings";
import { scopePath } from "@/lib/warnings";

export function RepoDetailView() {
  const { id } = useParams();
  const repoId = Number(id);
  const [tab, setTab] = useState<"overview" | "health" | "updates">("overview");

  const detail = useQuery({
    queryKey: ["repo", repoId],
    queryFn: () => unwrap(commands.getRepoDetail(repoId)),
    enabled: Number.isFinite(repoId),
  });

  const lastScan = useQuery({
    queryKey: ["latestScan"],
    queryFn: () => unwrap(commands.latestScanSummary()),
  });

  if (detail.isLoading) {
    return <p className="p-6 text-sm text-muted-foreground">Loading…</p>;
  }
  if (!detail.data) {
    return (
      <div className="p-6 text-sm text-muted-foreground">
        Repository not found. <Link to="/repos" className="underline">Back to list</Link>
      </div>
    );
  }

  const { repo, languages, submodules, technologies } = detail.data;
  const band = repo.healthBand ?? "unknown";
  const repoWarnings = (lastScan.data?.warnings ?? []).filter(
    (w) => scopePath(w) === repo.path,
  );

  return (
    <div>
      <Link
        to="/repos"
        className="mb-4 inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="size-4" />
        Repositories
      </Link>

      <PageHeader title={repo.name} description={repo.path} />

      {repoWarnings.length > 0 ? (
        <ScanWarnings
          warnings={repoWarnings}
          title="warnings while scanning this repository"
          className="mb-4"
        />
      ) : null}

      <div className="mb-4 flex gap-1 border-b">
        {(["overview", "health", "updates"] as const).map((t) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className={cn(
              "border-b-2 px-3 py-2 text-sm font-medium capitalize -mb-px",
              tab === t
                ? "border-primary text-foreground"
                : "border-transparent text-muted-foreground hover:text-foreground",
            )}
          >
            {t}
            {t === "health" && band !== "unknown" && repo.healthScore != null ? (
              <span className="ml-1.5 tabular-nums text-xs text-muted-foreground">
                {repo.healthScore}
              </span>
            ) : null}
          </button>
        ))}
      </div>

      {tab === "health" ? (
        <HealthTab detail={detail.data} />
      ) : tab === "updates" ? (
        <OutdatedTab repoId={repoId} />
      ) : (
        <div className="grid gap-4 md:grid-cols-2">
          <div className="md:col-span-2">
            <CategorySignals repo={repo} />
          </div>

          <Card title="Technologies">
            {technologies.length === 0 ? (
              <p className="text-sm text-muted-foreground">Nothing detected.</p>
            ) : (
              <ul className="flex flex-wrap gap-1.5">
                {technologies.map((t) => {
                  const confirmed = t.evidence.some((e) => e.startsWith("dependency:"));
                  return (
                    <li
                      key={t.tech}
                      title={t.evidence.join("\n")}
                      className={cn(
                        "rounded-md border px-2 py-1 text-xs",
                        confirmed
                          ? "bg-secondary font-medium"
                          : "border-dashed text-muted-foreground",
                      )}
                    >
                      {t.tech}
                      <span className="ml-1 text-[10px] uppercase text-muted-foreground/70">
                        {t.kind}
                      </span>
                    </li>
                  );
                })}
              </ul>
            )}
            <p className="mt-2 text-[11px] text-muted-foreground/70">
              Solid = confirmed by a dependency · dashed = marker file only.
            </p>
          </Card>

          <Card title="Git">
            {repo.isBare ? (
              <p className="text-sm text-muted-foreground">Bare repository — no working tree.</p>
            ) : (
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
                <Dt icon={GitBranch}>Branch</Dt>
                <dd>{repo.branch ?? "detached"}</dd>
                <Dt icon={GitCommitHorizontal}>Last commit</Dt>
                <dd className="truncate" title={repo.lastCommitSummary ?? ""}>
                  {repo.lastCommitSummary ?? "—"}
                </dd>
                <Dt>Commits (90d / total)</Dt>
                <dd className="tabular-nums">
                  {repo.commits90d ?? "—"} / {repo.commitsTotal ?? "—"}
                </dd>
                <Dt icon={Users}>Authors</Dt>
                <dd className="tabular-nums">{repo.authorCount ?? "—"}</dd>
                <Dt>Ahead / behind</Dt>
                <dd className="tabular-nums">
                  {repo.ahead ?? "—"} / {repo.behind ?? "—"}
                </dd>
                <Dt>Remote</Dt>
                <dd className="truncate">{repo.remoteUrl ?? "—"}</dd>
              </dl>
            )}
          </Card>

          <Card title="Languages">
            {languages.length === 0 ? (
              <p className="text-sm text-muted-foreground">No code counted.</p>
            ) : (
              <ul className="space-y-1.5">
                {languages.slice(0, 8).map((l) => {
                  const pct = l.percentage ?? 0;
                  return (
                    <li key={l.language} className="text-sm">
                      <div className="flex justify-between">
                        <span>{l.language}</span>
                        <span className="tabular-nums text-muted-foreground">
                          {pct.toFixed(1)}%
                        </span>
                      </div>
                      <div className="mt-0.5 h-1 overflow-hidden rounded-full bg-secondary">
                        <div className="h-full bg-primary" style={{ width: `${pct}%` }} />
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </Card>

          {submodules.length > 0 ? (
            <Card title={`Submodules (${submodules.length})`}>
              <ul className="space-y-1 text-sm">
                {submodules.map((sm) => (
                  <li key={sm.id} className="flex justify-between gap-2">
                    <span>{sm.name}</span>
                    <span className="truncate text-xs text-muted-foreground" title={sm.path}>
                      {sm.path}
                    </span>
                  </li>
                ))}
              </ul>
            </Card>
          ) : null}
        </div>
      )}
    </div>
  );
}

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-lg border bg-card p-4">
      <h2 className="mb-3 text-sm font-semibold">{title}</h2>
      {children}
    </section>
  );
}

function Dt({
  icon: Icon,
  children,
}: {
  icon?: typeof GitBranch;
  children: React.ReactNode;
}) {
  return (
    <dt className="inline-flex items-center gap-1.5 text-muted-foreground">
      {Icon ? <Icon className="size-3.5" /> : null}
      {children}
    </dt>
  );
}
