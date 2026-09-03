import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { RefreshCw, ShieldAlert } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { commands, unwrap } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useSyncNow, useSyncStatus } from "./use-sync";

function ago(iso: string | null | undefined): string {
  if (!iso) return "never";
  const d = Date.now() - new Date(iso).getTime();
  if (Number.isNaN(d)) return "never";
  const h = d / 3_600_000;
  if (h < 1) return "just now";
  if (h < 24) return `${Math.round(h)}h ago`;
  return `${Math.round(h / 24)}d ago`;
}

export function AdvisoriesView() {
  const status = useSyncStatus();
  const syncNow = useSyncNow();
  const s = status.data;
  const syncing = syncNow.isPending;

  return (
    <div>
      <PageHeader
        title="Advisories"
        description="OSV advisory database status and cross-repository impact."
        actions={
          <div className="flex gap-2">
            <Button
              variant="outline"
              onClick={() => syncNow.mutate("incremental")}
              disabled={syncing}
            >
              <RefreshCw className={cn("size-4", syncing && "animate-spin")} />
              Sync now
            </Button>
            <Button
              variant="ghost"
              onClick={() => syncNow.mutate("full")}
              disabled={syncing}
            >
              Full re-sync
            </Button>
          </div>
        }
      />

      {!s ? (
        <p className="text-sm text-muted-foreground">Loading…</p>
      ) : !s.everSynced ? (
        <div className="rounded-lg border border-unknown/40 bg-unknown/5 p-6">
          <p className="font-medium text-unknown">
            The advisory database has never been synced.
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            Until it syncs, repository health is shown as <b>unknown</b> — not
            healthy. Click <b>Sync now</b> to download the OSV database for the
            ecosystems your repositories use. Nothing about your code is sent;
            the whole database is downloaded and matched locally.
          </p>
        </div>
      ) : (
        <div className="space-y-6">
          <div className="grid gap-4 sm:grid-cols-3">
            <Stat label="Advisories stored" value={s.advisoryCount.toLocaleString()} />
            <Stat label="Last successful sync" value={ago(s.lastSuccess)} />
            <Stat
              label="Freshness"
              value={String(s.freshness)}
              tone={
                s.freshness === "Fresh"
                  ? "ok"
                  : s.freshness === "Never"
                    ? "unknown"
                    : "warn"
              }
            />
          </div>

          {s.lastError ? (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              Last sync attempt failed: {s.lastError} — the previous snapshot is
              still in use.
            </div>
          ) : null}

          <section>
            <h2 className="mb-2 text-sm font-semibold">By ecosystem</h2>
            <div className="overflow-hidden rounded-lg border">
              <table className="w-full text-sm">
                <thead className="bg-card text-xs uppercase tracking-wide text-muted-foreground">
                  <tr>
                    <th className="px-3 py-2 text-left">Ecosystem</th>
                    <th className="px-3 py-2 text-right">Advisories</th>
                    <th className="px-3 py-2 text-right">Compromises</th>
                    <th className="px-3 py-2 text-left">Coverage</th>
                    <th className="px-3 py-2 text-left">Synced</th>
                  </tr>
                </thead>
                <tbody className="divide-y">
                  {s.ecosystems.map((e) => (
                    <tr key={e.ecosystem}>
                      <td className="px-3 py-2 font-medium">{e.ecosystem}</td>
                      <td className="px-3 py-2 text-right tabular-nums">
                        {e.advisoryCount.toLocaleString()}
                      </td>
                      <td className="px-3 py-2 text-right tabular-nums text-compromise">
                        {e.compromiseCount.toLocaleString()}
                      </td>
                      <td className="px-3 py-2">
                        {e.malCoverage === "Thin" ? (
                          <span className="inline-flex items-center gap-1 rounded bg-warn/15 px-1.5 py-0.5 text-xs text-warn">
                            <ShieldAlert className="size-3" />
                            thin — spot reports only
                          </span>
                        ) : (
                          <span className="text-xs text-muted-foreground">
                            comprehensive
                          </span>
                        )}
                      </td>
                      <td className="px-3 py-2 text-xs text-muted-foreground">
                        {ago(e.lastSuccess)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p className="mt-2 text-xs text-muted-foreground">
              For <b>crates.io</b> and <b>Go</b>, OSV tracks only a handful of
              hand-filed malicious-package reports. "No compromise findings"
              there means the known cases were checked — not that a repository
              is definitely clean.
            </p>
          </section>

          <ImpactLookup />
          <LiveQuery />
        </div>
      )}
    </div>
  );
}

function LiveQuery() {
  const [eco, setEco] = useState("npm");
  const [name, setName] = useState("");
  const [version, setVersion] = useState("");
  const q = useMutation({
    mutationFn: () =>
      unwrap(commands.liveQuery(eco, name.trim(), version.trim())),
  });

  return (
    <section className="rounded-lg border border-warn/30 bg-warn/5 p-4">
      <h2 className="text-sm font-semibold">Check a single package live</h2>
      <p className="mt-1 text-xs text-muted-foreground">
        This sends the package name and version to <b>api.osv.dev</b> — the one
        exception to repo-radar's "nothing about your code leaves the machine"
        rule. Use it to spot-check a package without a full sync.
      </p>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <select
          value={eco}
          onChange={(e) => setEco(e.target.value)}
          className="h-9 rounded-md border bg-background px-2 text-sm"
        >
          {["npm", "PyPI", "crates.io", "Go"].map((x) => (
            <option key={x}>{x}</option>
          ))}
        </select>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="package name"
          className="h-9 w-48 rounded-md border bg-background px-3 text-sm"
        />
        <input
          value={version}
          onChange={(e) => setVersion(e.target.value)}
          placeholder="version"
          className="h-9 w-32 rounded-md border bg-background px-3 text-sm"
        />
        <Button
          size="sm"
          variant="outline"
          onClick={() => q.mutate()}
          disabled={!name.trim() || !version.trim() || q.isPending}
        >
          Query OSV
        </Button>
      </div>
      {q.data ? (
        q.data.advisoryIds.length > 0 ? (
          <p className="mt-2 text-sm text-vulnerability">
            {q.data.advisoryIds.join(", ")}
          </p>
        ) : (
          <p className="mt-2 text-sm text-ok">No advisories for that version.</p>
        )
      ) : null}
      {q.error ? (
        <p className="mt-2 text-sm text-destructive">
          {(q.error as Error).message}
        </p>
      ) : null}
    </section>
  );
}

function Stat({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "ok" | "warn" | "unknown";
}) {
  return (
    <div className="rounded-lg border bg-card p-4">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div
        className={cn(
          "mt-1 text-2xl font-semibold",
          tone === "ok" && "text-ok",
          tone === "warn" && "text-warn",
          tone === "unknown" && "text-unknown",
        )}
      >
        {value}
      </div>
    </div>
  );
}

function ImpactLookup() {
  const [id, setId] = useState("");
  const impact = useQuery({
    queryKey: ["advisoryImpact", id],
    queryFn: () => unwrap(commands.listAdvisoryImpact(id.trim())),
    enabled: id.trim().length > 3,
  });

  return (
    <section>
      <h2 className="mb-2 text-sm font-semibold">Cross-repository impact</h2>
      <input
        value={id}
        onChange={(e) => setId(e.target.value)}
        placeholder="Advisory id, e.g. GHSA-… or MAL-…"
        className="h-9 w-80 rounded-md border bg-background px-3 text-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
      />
      {impact.data && impact.data.length > 0 ? (
        <ul className="mt-2 space-y-1 text-sm">
          {impact.data.map((r) => (
            <li key={r.repoId}>{r.repoName}</li>
          ))}
        </ul>
      ) : id.trim().length > 3 && !impact.isFetching ? (
        <p className="mt-2 text-sm text-muted-foreground">
          No repositories currently affected by {id.trim()}.
        </p>
      ) : null}
    </section>
  );
}
