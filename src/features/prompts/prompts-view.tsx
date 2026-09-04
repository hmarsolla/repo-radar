import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { save } from "@tauri-apps/plugin-dialog";
import { Check, ClipboardCopy, Download, Sparkles } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { commands, unwrap } from "@/lib/ipc";
import type {
  GeneratedPrompt,
  RepoListItem,
  ScopeContext,
  TemplateInfo,
} from "@/bindings";
import { Onboarding, useHasScanRoot } from "@/features/onboarding/onboarding";
import { FileTree } from "./file-tree";

type ScopeKind = ScopeContext["kind"];

export function PromptsView() {
  const hasRoot = useHasScanRoot();

  const templates = useQuery({
    queryKey: ["promptTemplates"],
    queryFn: () => unwrap(commands.listPromptTemplates()),
  });
  const repos = useQuery({
    queryKey: ["repos", "prompt-picker"],
    queryFn: () => unwrap(commands.listRepos({})),
    enabled: hasRoot.data === true,
  });
  const settings = useQuery({
    queryKey: ["settings"],
    queryFn: () => unwrap(commands.getSettings()),
  });

  const [templateId, setTemplateId] = useState<string | null>(null);
  const [selectedRepos, setSelectedRepos] = useState<number[]>([]);
  const [scopeKind, setScopeKind] = useState<ScopeKind>("wholeRepo");
  const [dirPath, setDirPath] = useState("");
  const [diffText, setDiffText] = useState("");
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [result, setResult] = useState<GeneratedPrompt | null>(null);
  const [generating, setGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const template: TemplateInfo | undefined = useMemo(
    () => templates.data?.find((t) => t.id === templateId),
    [templates.data, templateId],
  );
  const multi = template?.arity === "multi";
  const primaryRepo = selectedRepos[0];
  const usesFiles = !!template?.usesFiles && !multi && scopeKind !== "diff";

  const listing = useQuery({
    queryKey: ["promptFiles", primaryRepo],
    queryFn: () => unwrap(commands.promptFileListing(primaryRepo!)),
    enabled: usesFiles && primaryRepo != null,
  });

  // Keep selection consistent when the shape of the request changes.
  useEffect(() => {
    if (!multi && selectedRepos.length > 1) setSelectedRepos((r) => r.slice(0, 1));
    setResult(null);
    setError(null);
  }, [templateId, multi]); // eslint-disable-line react-hooks/exhaustive-deps

  // Auto-fill the file selection for the non-manual scopes.
  const allFiles = useMemo(() => listing.data?.files ?? [], [listing.data]);
  useEffect(() => {
    if (!usesFiles) return;
    if (scopeKind === "wholeRepo") {
      setSelectedPaths(
        new Set(allFiles.filter((f) => !f.isDir && f.excluded === null).map((f) => f.path)),
      );
    } else if (scopeKind === "directory") {
      const prefix = dirPath.replace(/^\/+|\/+$/g, "");
      setSelectedPaths(
        new Set(
          allFiles
            .filter(
              (f) =>
                !f.isDir &&
                f.excluded === null &&
                (prefix === "" || f.path === prefix || f.path.startsWith(prefix + "/")),
            )
            .map((f) => f.path),
        ),
      );
    }
  }, [scopeKind, dirPath, listing.data, usesFiles]); // eslint-disable-line react-hooks/exhaustive-deps

  const budget = settings.data?.tokenBudget ?? 128_000;
  const liveTokens = useMemo(() => {
    if (scopeKind === "diff") return Math.ceil(diffText.length / 4);
    const bytes = allFiles
      .filter((f) => selectedPaths.has(f.path))
      .reduce((n, f) => n + f.bytes, 0);
    return Math.ceil(bytes / 4);
  }, [allFiles, selectedPaths, scopeKind, diffText]);

  if (hasRoot.isLoading) {
    return <p className="p-6 text-sm text-muted-foreground">Loading…</p>;
  }
  if (!hasRoot.data) return <Onboarding />;

  const repoList = repos.data ?? [];

  function toggleRepo(id: number) {
    setResult(null);
    setSelectedRepos((cur) => {
      if (multi) {
        return cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id];
      }
      return [id];
    });
  }

  function buildScope(): ScopeContext {
    switch (scopeKind) {
      case "directory":
        return { kind: "directory", path: dirPath.replace(/^\/+|\/+$/g, "") };
      case "files":
        return { kind: "files", paths: [...selectedPaths] };
      case "diff":
        return { kind: "diff", description: diffText };
      default:
        return { kind: "wholeRepo" };
    }
  }

  async function generate() {
    setGenerating(true);
    setError(null);
    try {
      const res = await unwrap(
        commands.generatePrompt({
          templateId: templateId!,
          repoIds: selectedRepos,
          scope: buildScope(),
          selectedPaths: usesFiles ? [...selectedPaths] : [],
        }),
      );
      setResult(res);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setGenerating(false);
    }
  }

  async function copyPrompt() {
    if (!result) return;
    await writeText(result.prompt);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  async function exportPrompt() {
    if (!result) return;
    const path = await save({
      defaultPath: `${templateId ?? "prompt"}.md`,
      filters: [{ name: "Text", extensions: ["md", "txt"] }],
    });
    if (typeof path !== "string") return;
    try {
      await unwrap(commands.exportPrompt(path, result.prompt));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const canGenerate =
    !!templateId &&
    selectedRepos.length >= (multi ? 2 : 1) &&
    (!template?.usesFiles ||
      scopeKind === "wholeRepo" ||
      scopeKind === "files" ||
      (scopeKind === "directory" && dirPath.trim().length > 0) ||
      (scopeKind === "diff" && diffText.trim().length > 0)) &&
    !generating;

  const overBudget = (result?.estimatedTokens ?? liveTokens) > budget;

  return (
    <div className="max-w-4xl">
      <PageHeader
        title="Prompts"
        description="Assemble a prompt from a template plus your repositories. Nothing leaves this machine until you copy or export it — you see the full text first."
      />

      <div className="grid gap-6">
        {/* 1 — template */}
        <Section step={1} title="Template">
          {templates.isLoading ? (
            <p className="text-sm text-muted-foreground">Loading…</p>
          ) : (
            <div className="grid gap-2 sm:grid-cols-2">
              {templates.data?.map((t) => (
                <button
                  key={t.id}
                  type="button"
                  onClick={() => setTemplateId(t.id)}
                  className={cn(
                    "rounded-lg border p-3 text-left transition-colors",
                    templateId === t.id
                      ? "border-primary bg-primary/5"
                      : "hover:bg-accent/50",
                  )}
                >
                  <div className="flex items-center gap-2 text-sm font-medium">
                    {t.name}
                    <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-normal uppercase text-muted-foreground">
                      {t.source === "builtIn" ? "built-in" : "yours"}
                    </span>
                    <span className="text-[10px] font-normal text-muted-foreground">
                      {t.arity === "multi" ? "· multi-repo" : "· one repo"}
                    </span>
                  </div>
                  <p className="mt-1 text-xs text-muted-foreground">{t.description}</p>
                </button>
              ))}
            </div>
          )}
        </Section>

        {/* 2 — repositories */}
        <Section
          step={2}
          title={multi ? "Repositories (pick two or more)" : "Repository"}
          disabled={!templateId}
        >
          <div className="max-h-64 overflow-auto rounded-lg border bg-card divide-y">
            {repoList.map((r: RepoListItem) => {
              const on = selectedRepos.includes(r.id);
              return (
                <label
                  key={r.id}
                  className="flex cursor-pointer items-center gap-2 px-3 py-2 text-sm hover:bg-accent/50"
                >
                  <input
                    type={multi ? "checkbox" : "radio"}
                    name="prompt-repo"
                    checked={on}
                    onChange={() => toggleRepo(r.id)}
                  />
                  <span className="font-medium">{r.name}</span>
                  {r.primaryLanguage ? (
                    <span className="text-xs text-muted-foreground">
                      {r.primaryLanguage}
                    </span>
                  ) : null}
                  <span className="ml-auto truncate text-xs text-muted-foreground" title={r.path}>
                    {r.path}
                  </span>
                </label>
              );
            })}
            {repoList.length === 0 ? (
              <p className="px-3 py-3 text-sm text-muted-foreground">
                No repositories scanned yet.
              </p>
            ) : null}
          </div>
        </Section>

        {/* 3 — scope + files */}
        {template?.usesFiles ? (
          <Section step={3} title="What to include" disabled={selectedRepos.length === 0}>
            <div className="mb-3 flex flex-wrap gap-1">
              {(
                [
                  ["wholeRepo", "Whole repo"],
                  ["directory", "Directory"],
                  ["files", "Pick files"],
                  ["diff", "Paste a diff"],
                ] as [ScopeKind, string][]
              ).map(([k, label]) => (
                <button
                  key={k}
                  type="button"
                  disabled={multi && k !== "wholeRepo"}
                  onClick={() => {
                    setScopeKind(k);
                    setResult(null);
                  }}
                  className={cn(
                    "rounded-md border px-2.5 py-1 text-xs font-medium disabled:opacity-40",
                    scopeKind === k
                      ? "border-primary bg-primary/5"
                      : "hover:bg-accent/50",
                  )}
                >
                  {label}
                </button>
              ))}
            </div>

            {scopeKind === "directory" ? (
              <input
                value={dirPath}
                onChange={(e) => setDirPath(e.target.value)}
                placeholder="src/  (repo-relative path)"
                className="mb-3 h-9 w-full rounded-md border bg-background px-3 font-mono text-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
              />
            ) : null}

            {scopeKind === "diff" ? (
              <textarea
                value={diffText}
                onChange={(e) => setDiffText(e.target.value)}
                placeholder="Paste `git diff` output here."
                rows={10}
                className="w-full rounded-md border bg-background p-3 font-mono text-xs outline-none focus-visible:ring-1 focus-visible:ring-ring"
              />
            ) : usesFiles ? (
              listing.isLoading ? (
                <p className="text-sm text-muted-foreground">Reading the repository…</p>
              ) : (
                <>
                  {listing.data?.truncated ? (
                    <p className="mb-2 text-[11px] text-amber-600 dark:text-amber-500">
                      This repository has more files than the picker will list; some are
                      not shown.
                    </p>
                  ) : null}
                  <FileTree
                    files={allFiles}
                    selected={selectedPaths}
                    onToggle={(p, on) => {
                      setResult(null);
                      setSelectedPaths((s) => {
                        const next = new Set(s);
                        if (on) next.add(p);
                        else next.delete(p);
                        return next;
                      });
                    }}
                    onBulk={(paths, on) => {
                      setResult(null);
                      setSelectedPaths((s) => {
                        const next = new Set(s);
                        for (const p of paths) {
                          if (on) next.add(p);
                          else next.delete(p);
                        }
                        return next;
                      });
                    }}
                  />
                  {scopeKind !== "files" ? (
                    <p className="mt-1 text-[11px] text-muted-foreground">
                      {scopeKind === "wholeRepo"
                        ? "Every selectable file is included. Switch to “Pick files” to choose."
                        : "Every selectable file under the directory is included."}
                    </p>
                  ) : null}
                </>
              )
            ) : null}
          </Section>
        ) : null}

        {/* 4 — generate */}
        <Section step={template?.usesFiles ? 4 : 3} title="Generate" disabled={!canGenerate && !result}>
          <div className="flex flex-wrap items-center gap-3">
            <Button onClick={generate} disabled={!canGenerate}>
              <Sparkles />
              {generating ? "Generating…" : "Generate prompt"}
            </Button>
            <span
              className={cn(
                "text-sm tabular-nums",
                overBudget ? "text-amber-600 dark:text-amber-500" : "text-muted-foreground",
              )}
            >
              ≈ {(result?.estimatedTokens ?? liveTokens).toLocaleString()} tokens
              <span className="text-muted-foreground"> / {budget.toLocaleString()} budget (estimate)</span>
            </span>
          </div>
          {overBudget ? (
            <p className="mt-2 text-xs text-amber-600 dark:text-amber-500">
              Over the configured budget. This does not block generation — your target model
              may have a larger context.
            </p>
          ) : null}
          {error ? (
            <div className="mt-3 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {error}
            </div>
          ) : null}
        </Section>
      </div>

      {/* preview */}
      {result ? (
        <div className="mt-6">
          <div className="mb-2 flex items-center gap-2">
            <h2 className="text-sm font-semibold">Preview — {result.templateName}</h2>
            <span className="text-xs text-muted-foreground">
              {result.estimatedTokens.toLocaleString()} tokens (estimate) ·{" "}
              {result.includedFiles} file{result.includedFiles === 1 ? "" : "s"} embedded
            </span>
            <div className="ml-auto flex gap-2">
              <Button size="sm" variant="outline" onClick={copyPrompt}>
                {copied ? <Check /> : <ClipboardCopy />}
                {copied ? "Copied" : "Copy"}
              </Button>
              <Button size="sm" variant="outline" onClick={exportPrompt}>
                <Download />
                Export…
              </Button>
            </div>
          </div>

          {result.skippedFiles.length > 0 ? (
            <details className="mb-2 rounded-md border bg-card px-3 py-2 text-xs">
              <summary className="cursor-pointer text-muted-foreground">
                {result.skippedFiles.length} selected file
                {result.skippedFiles.length === 1 ? "" : "s"} were not embedded
              </summary>
              <ul className="mt-2 space-y-0.5 font-mono">
                {result.skippedFiles.map((s) => (
                  <li key={s.path} className="flex justify-between gap-3">
                    <span className="truncate">{s.path}</span>
                    <span className="shrink-0 text-muted-foreground">{s.reason.reason}</span>
                  </li>
                ))}
              </ul>
            </details>
          ) : null}

          <pre className="max-h-[36rem] overflow-auto rounded-lg border bg-card p-4 text-xs leading-relaxed whitespace-pre-wrap">
            {result.prompt}
          </pre>
          <p className="mt-2 text-[11px] text-muted-foreground">
            This is the exact text that will be copied or written to disk. It may contain
            proprietary source — review it before sending it to a third party. Export is
            blocked from writing inside a scanned repository.
          </p>
        </div>
      ) : null}
    </div>
  );
}

function Section({
  step,
  title,
  disabled,
  children,
}: {
  step: number;
  title: string;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <section className={cn(disabled && "pointer-events-none opacity-50")}>
      <h2 className="mb-2 flex items-center gap-2 text-sm font-semibold">
        <span className="flex size-5 items-center justify-center rounded-full bg-secondary text-xs text-secondary-foreground">
          {step}
        </span>
        {title}
      </h2>
      {children}
    </section>
  );
}
