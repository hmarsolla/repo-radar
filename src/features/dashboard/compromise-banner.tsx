import { Link } from "react-router-dom";
import { AlertOctagon } from "lucide-react";

import type { RepoRef } from "@/bindings";

/**
 * CompromiseBanner (FR-6.3, DESIGN §14.3). Rendered on the Dashboard **only**
 * when at least one repo has a confirmed-compromise finding. A banner that is
 * always present is furniture; this one has to mean something when it shows.
 */
export function CompromiseBanner({ repos }: { repos: RepoRef[] }) {
  if (repos.length === 0) return null;

  const total = repos.reduce((n, r) => n + r.compromiseCount, 0);

  return (
    <div
      role="alert"
      className="mb-4 rounded-lg border border-compromise/50 bg-compromise/10 p-4"
    >
      <div className="flex items-center gap-2 font-semibold text-compromise">
        <AlertOctagon className="size-5" />
        {total} confirmed compromise{total === 1 ? "" : "s"} across{" "}
        {repos.length} {repos.length === 1 ? "repository" : "repositories"}
      </div>
      <p className="mt-1 text-sm text-muted-foreground">
        A backdoored or malicious package is a different class of problem from a
        CVE — treat these first.
      </p>
      <ul className="mt-2 flex flex-wrap gap-1.5">
        {repos.map((r) => (
          <li key={r.id}>
            <Link
              to={`/repos/${r.id}`}
              className="inline-flex items-center gap-1 rounded-md border border-compromise/40 bg-card px-2 py-1 text-xs font-medium hover:bg-compromise/10"
            >
              {r.name}
              <span className="tabular-nums text-compromise">
                {r.compromiseCount}
              </span>
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}
