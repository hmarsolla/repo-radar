import { useMemo, useState } from "react";

import { cn } from "@/lib/utils";
import type { ExclusionReason, SelectableFile } from "@/bindings";

const ROW_CAP = 600;

function reasonLabel(ex: ExclusionReason): string {
  switch (ex.reason) {
    case "pruned":
      return "pruned dir";
    case "gitignored":
      return "gitignored";
    case "extensionExcluded":
      return `.${ex.ext} — excluded in settings`;
    case "oversized":
      return `${Math.round(ex.bytes / 1024)} KB — over limit`;
    case "binary":
      return "binary";
    case "unreadable":
      return `unreadable — ${ex.detail}`;
  }
}

function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function FileTree({
  files,
  selected,
  onToggle,
  onBulk,
}: {
  files: SelectableFile[];
  selected: Set<string>;
  onToggle: (path: string, on: boolean) => void;
  onBulk: (paths: string[], on: boolean) => void;
}) {
  const [filter, setFilter] = useState("");

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return q ? files.filter((f) => f.path.toLowerCase().includes(q)) : files;
  }, [files, filter]);

  const visibleSelectable = useMemo(
    () => filtered.filter((f) => !f.isDir && f.excluded === null).map((f) => f.path),
    [filtered],
  );
  const shown = filtered.slice(0, ROW_CAP);
  const selectableCount = files.filter((f) => !f.isDir && f.excluded === null).length;

  return (
    <div className="rounded-lg border bg-card">
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter paths"
          className="h-8 flex-1 rounded-md border bg-background px-2 text-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
        <button
          type="button"
          onClick={() => onBulk(visibleSelectable, true)}
          className="text-xs text-muted-foreground hover:text-foreground"
        >
          Select {filter ? "shown" : "all"}
        </button>
        <span className="text-muted-foreground/50">·</span>
        <button
          type="button"
          onClick={() => onBulk(visibleSelectable, false)}
          className="text-xs text-muted-foreground hover:text-foreground"
        >
          None
        </button>
      </div>

      <div className="max-h-[22rem] overflow-auto py-1 font-mono text-xs">
        {shown.map((f) => {
          const depth = f.path.split("/").length - 1;
          const name = f.path.split("/").pop() ?? f.path;
          const disabled = f.isDir || f.excluded !== null;
          return (
            <label
              key={f.path}
              className={cn(
                "flex items-center gap-2 px-3 py-0.5",
                disabled ? "opacity-45" : "hover:bg-accent/50 cursor-pointer",
              )}
              style={{ paddingLeft: `${0.75 + depth * 0.9}rem` }}
              title={f.path}
            >
              <input
                type="checkbox"
                className="size-3.5"
                disabled={disabled}
                checked={selected.has(f.path)}
                onChange={(e) => onToggle(f.path, e.target.checked)}
              />
              <span className={cn("truncate", f.isDir && "font-semibold")}>
                {name}
                {f.isDir ? "/" : ""}
              </span>
              {f.excluded ? (
                <span className="ml-auto shrink-0 rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                  {reasonLabel(f.excluded)}
                </span>
              ) : (
                <span className="ml-auto shrink-0 text-[10px] text-muted-foreground/60">
                  {prettyBytes(f.bytes)}
                </span>
              )}
            </label>
          );
        })}
        {filtered.length > ROW_CAP ? (
          <p className="px-3 py-2 text-[11px] text-muted-foreground">
            Showing {ROW_CAP} of {filtered.length}. Narrow the filter to see more.
          </p>
        ) : null}
        {filtered.length === 0 ? (
          <p className="px-3 py-2 text-[11px] text-muted-foreground">No matching paths.</p>
        ) : null}
      </div>

      <div className="border-t px-3 py-1.5 text-[11px] text-muted-foreground">
        {selected.size} selected · {selectableCount} selectable · excluded files are shown
        with the reason and cannot be picked.
      </div>
    </div>
  );
}
