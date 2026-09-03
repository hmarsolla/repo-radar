import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Play } from "lucide-react";
import {
  Bar,
  BarChart,
  Cell,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { commands, unwrap } from "@/lib/ipc";
import { useScan } from "@/features/scan/scan-provider";
import { Onboarding, useHasScanRoot } from "@/features/onboarding/onboarding";
import { CompromiseBanner } from "@/features/dashboard/compromise-banner";
import type { Bucket } from "@/bindings";

/**
 * Dashboard (PRD §6). Repo count, health histogram, category donut, language
 * bar, the stalest and worst-health repos, and — only when it means something
 * — the compromise banner (M3-6 / M3-7). Charts update on scan completion via
 * the `["dashboard"]` query key invalidated in the scan/sync providers.
 */
export function DashboardView() {
  const hasRoot = useHasScanRoot();
  const scan = useScan();

  const stats = useQuery({
    queryKey: ["dashboard"],
    queryFn: () => unwrap(commands.dashboardStats()),
    enabled: hasRoot.data === true,
  });

  if (hasRoot.isLoading) {
    return <p className="p-6 text-sm text-muted-foreground">Loading…</p>;
  }
  if (!hasRoot.data) {
    return <Onboarding />;
  }

  const s = stats.data;

  return (
    <div>
      <PageHeader
        title="Dashboard"
        description="Fleet overview — health distribution, categories, and the repos that need attention."
        actions={
          <Button onClick={() => scan.start()} disabled={scan.running}>
            <Play />
            {scan.running ? "Scanning…" : "Scan now"}
          </Button>
        }
      />

      {!s ? (
        <p className="text-sm text-muted-foreground">
          {stats.isLoading ? "Loading…" : "No data yet — run a scan."}
        </p>
      ) : (
        <>
          <CompromiseBanner repos={s.compromised} />

          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            <Stat label="Repositories" value={s.repoCount} />
            <Stat label="Dirty working trees" value={s.dirtyCount} />
            <Stat
              label="Compromised repos"
              value={s.compromised.length}
              tone={s.compromised.length > 0 ? "compromise" : undefined}
            />

            <Panel title="Health distribution" className="lg:col-span-2">
              <HealthChart data={s.healthDistribution} />
            </Panel>
            <Panel title="Categories">
              <CategoryChart data={s.categoryDistribution} />
            </Panel>

            <Panel title="Languages" className="lg:col-span-2">
              <LanguageChart data={s.languageDistribution} />
            </Panel>
            <Panel title="Freshest vs. stalest">
              <p className="text-xs text-muted-foreground">
                Oldest last-commit dates in the fleet.
              </p>
              <RepoList
                rows={s.stalest.map((r) => ({
                  id: r.id,
                  name: r.name,
                  right: relativeTime(r.lastCommitAt),
                }))}
              />
            </Panel>

            <Panel title="Worst health" className="lg:col-span-3">
              {s.worstHealth.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No health scores yet — sync advisories, then scan.
                </p>
              ) : (
                <RepoList
                  rows={s.worstHealth.map((r) => ({
                    id: r.id,
                    name: r.name,
                    right: `${r.healthScore ?? "—"} · ${r.healthBand ?? "unknown"}`,
                    tone: r.compromiseCount > 0 ? "compromise" : bandTone(r.healthBand),
                  }))}
                />
              )}
            </Panel>
          </div>
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Charts
// ---------------------------------------------------------------------------

const BAND_COLOR: Record<string, string> = {
  unknown: "var(--unknown)",
  critical: "var(--compromise)",
  poor: "var(--vulnerability)",
  fair: "var(--warn)",
  good: "var(--ok)",
  excellent: "var(--ok)",
};

const CATEGORY_COLORS: Record<string, string> = {
  Frontend: "#6366f1",
  Backend: "#0ea5e9",
  Fullstack: "#8b5cf6",
  Mobile: "#ec4899",
  DevOps: "#f59e0b",
  DataMl: "#10b981",
  Library: "#14b8a6",
  Cli: "#64748b",
  Docs: "#a855f7",
  Unknown: "var(--unknown)",
};

function HealthChart({ data }: { data: Bucket[] }) {
  const rows = data.map((b) => ({ ...b, label: cap(b.label) }));
  return (
    <ResponsiveContainer width="100%" height={200}>
      <BarChart data={rows} margin={{ top: 8, right: 8, bottom: 0, left: -16 }}>
        <XAxis
          dataKey="label"
          tick={{ fontSize: 11, fill: "var(--muted-foreground)" }}
          axisLine={false}
          tickLine={false}
        />
        <YAxis
          allowDecimals={false}
          tick={{ fontSize: 11, fill: "var(--muted-foreground)" }}
          axisLine={false}
          tickLine={false}
        />
        <Tooltip
          cursor={{ fill: "var(--muted)" }}
          contentStyle={tooltipStyle}
          labelStyle={{ color: "var(--foreground)" }}
        />
        <Bar dataKey="count" radius={[4, 4, 0, 0]}>
          {data.map((b) => (
            <Cell key={b.label} fill={BAND_COLOR[b.label] ?? "var(--unknown)"} />
          ))}
        </Bar>
      </BarChart>
    </ResponsiveContainer>
  );
}

function CategoryChart({ data }: { data: Bucket[] }) {
  const rows = data.filter((b) => b.count > 0);
  if (rows.length === 0) {
    return <Empty>No classifications yet.</Empty>;
  }
  return (
    <ResponsiveContainer width="100%" height={200}>
      <PieChart>
        <Pie
          data={rows}
          dataKey="count"
          nameKey="label"
          innerRadius={45}
          outerRadius={80}
          paddingAngle={2}
          stroke="var(--card)"
        >
          {rows.map((b) => (
            <Cell key={b.label} fill={CATEGORY_COLORS[b.label] ?? "#64748b"} />
          ))}
        </Pie>
        <Tooltip contentStyle={tooltipStyle} labelStyle={{ color: "var(--foreground)" }} />
      </PieChart>
    </ResponsiveContainer>
  );
}

function LanguageChart({ data }: { data: Bucket[] }) {
  if (data.length === 0) return <Empty>No code counted yet.</Empty>;
  return (
    <ResponsiveContainer width="100%" height={Math.max(120, data.length * 28)}>
      <BarChart
        data={data}
        layout="vertical"
        margin={{ top: 4, right: 16, bottom: 4, left: 8 }}
      >
        <XAxis type="number" hide />
        <YAxis
          type="category"
          dataKey="label"
          width={92}
          tick={{ fontSize: 11, fill: "var(--muted-foreground)" }}
          axisLine={false}
          tickLine={false}
        />
        <Tooltip
          cursor={{ fill: "var(--muted)" }}
          contentStyle={tooltipStyle}
          labelStyle={{ color: "var(--foreground)" }}
          formatter={(v) => [`${Number(v).toLocaleString()} lines`, "Code"]}
        />
        <Bar dataKey="count" fill="#6366f1" radius={[0, 4, 4, 0]} />
      </BarChart>
    </ResponsiveContainer>
  );
}

const tooltipStyle = {
  background: "var(--card)",
  border: "1px solid var(--border)",
  borderRadius: 8,
  fontSize: 12,
} as const;

// ---------------------------------------------------------------------------
// Layout primitives
// ---------------------------------------------------------------------------

function Stat({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone?: "compromise";
}) {
  return (
    <div className="flex h-40 flex-col rounded-lg border bg-card p-4">
      <span className="text-sm font-medium text-muted-foreground">{label}</span>
      <span
        className={
          "m-auto text-5xl font-semibold tabular-nums" +
          (tone === "compromise" ? " text-compromise" : "")
        }
      >
        {value}
      </span>
    </div>
  );
}

function Panel({
  title,
  children,
  className = "",
}: {
  title: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <section className={"rounded-lg border bg-card p-4 " + className}>
      <h2 className="mb-3 text-sm font-semibold">{title}</h2>
      {children}
    </section>
  );
}

function RepoList({
  rows,
}: {
  rows: { id: number; name: string; right: string; tone?: string }[];
}) {
  if (rows.length === 0) {
    return <p className="mt-2 text-sm text-muted-foreground">Nothing to show.</p>;
  }
  return (
    <ul className="mt-2 divide-y">
      {rows.map((r) => (
        <li key={r.id} className="flex items-center justify-between gap-3 py-1.5 text-sm">
          <Link to={`/repos/${r.id}`} className="truncate hover:underline">
            {r.name}
          </Link>
          <span
            className={
              "shrink-0 tabular-nums text-xs " +
              (r.tone ?? "text-muted-foreground")
            }
          >
            {r.right}
          </span>
        </li>
      ))}
    </ul>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex h-[200px] items-center justify-center text-xs text-muted-foreground/70">
      {children}
    </div>
  );
}

// ---------------------------------------------------------------------------

function bandTone(band: string | null): string {
  switch (band) {
    case "critical":
      return "text-compromise";
    case "poor":
      return "text-vulnerability";
    case "fair":
      return "text-warn";
    case "good":
    case "excellent":
      return "text-ok";
    default:
      return "text-unknown";
  }
}

function cap(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function relativeTime(iso: string | null): string {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "—";
  const diff = Date.now() - then;
  const day = 86_400_000;
  if (diff < day) return "today";
  if (diff < 2 * day) return "yesterday";
  if (diff < 30 * day) return `${Math.round(diff / day)}d ago`;
  if (diff < 365 * day) return `${Math.round(diff / (30 * day))}mo ago`;
  return `${Math.round(diff / (365 * day))}y ago`;
}
