import { useParams, Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, GitBranch, GitCommitHorizontal, Users } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { commands, unwrap } from "@/lib/ipc";

export function RepoDetailView() {
  const { id } = useParams();
  const repoId = Number(id);

  const detail = useQuery({
    queryKey: ["repo", repoId],
    queryFn: () => unwrap(commands.getRepoDetail(repoId)),
    enabled: Number.isFinite(repoId),
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

  const { repo, languages, submodules } = detail.data;

  return (
    <div>
      <Link
        to="/repos"
        className="mb-4 inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="size-4" />
        Repositories
      </Link>

      <PageHeader
        title={repo.name}
        description={repo.path}
      />

      <div className="grid gap-4 md:grid-cols-2">
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
              <Dt>Working tree</Dt>
              <dd className="tabular-nums">
                {(repo.dirtyModified ?? 0) + (repo.dirtyStaged ?? 0) + (repo.dirtyUntracked ?? 0) === 0
                  ? "clean"
                  : `${repo.dirtyModified ?? 0} modified · ${repo.dirtyStaged ?? 0} staged · ${repo.dirtyUntracked ?? 0} untracked`}
              </dd>
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
              {submodules.map((s) => (
                <li key={s.id} className="flex justify-between">
                  <span>{s.name}</span>
                  <span className="truncate text-xs text-muted-foreground" title={s.path}>
                    {s.path}
                  </span>
                </li>
              ))}
            </ul>
          </Card>
        ) : null}

        <Card title="Health & dependencies">
          <p className="text-sm text-muted-foreground">
            Dependency inventory and health scoring arrive in M2.
          </p>
        </Card>
      </div>
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
