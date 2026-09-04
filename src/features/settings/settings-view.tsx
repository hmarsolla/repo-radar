import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ChevronDown,
  ChevronUp,
  FolderOpen,
  FolderPlus,
  Trash2,
  X,
} from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { commands, unwrap } from "@/lib/ipc";
import type { Settings } from "@/bindings";
import { ThemeToggle } from "./theme-toggle";

const DEFAULT_SETTINGS: Required<Omit<Settings, "theme">> = {
  pruneList: [],
  excludedExtensions: [],
  syncIntervalHours: 24,
  tokenBudget: 128_000,
};

export function SettingsView() {
  const qc = useQueryClient();

  const settingsQ = useQuery({
    queryKey: ["settings"],
    queryFn: () => unwrap(commands.getSettings()),
  });

  const save = useMutation({
    mutationFn: (next: Settings) => unwrap(commands.setSettings(next)),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["settings"] }),
  });

  const s = { ...DEFAULT_SETTINGS, ...settingsQ.data };
  const patch = (part: Partial<Settings>) =>
    save.mutate({ ...settingsQ.data, ...part });

  return (
    <div className="max-w-2xl space-y-8">
      <PageHeader
        title="Settings"
        description="Everything here is stored in your OS config directory — never in a scanned repository or the app's source tree."
      />

      <ScanRootsSection />

      <Section title="Discovery">
        <PruneDirs
          extra={s.pruneList}
          onChange={(pruneList) => patch({ pruneList })}
        />
      </Section>

      <Section title="Advisory sync">
        <fieldset className="flex gap-4 text-sm" disabled={save.isPending}>
          {[
            { label: "Daily", hours: 24 },
            { label: "Manual only", hours: 0 },
          ].map((opt) => (
            <label key={opt.hours} className="flex items-center gap-2">
              <input
                type="radio"
                name="syncInterval"
                checked={s.syncIntervalHours === opt.hours}
                onChange={() => patch({ syncIntervalHours: opt.hours })}
              />
              {opt.label}
            </label>
          ))}
        </fieldset>
        <p className="mt-2 text-xs text-muted-foreground">
          The scheduled sync fetches only the advisory database from OSV. Manual
          syncs from the Advisories screen always work regardless of this
          setting.
        </p>
      </Section>

      <Section title="Prompts">
        <label className="flex items-center gap-3 text-sm">
          <span className="w-32 text-muted-foreground">Token budget</span>
          <input
            type="number"
            min={1000}
            step={1000}
            defaultValue={s.tokenBudget}
            key={s.tokenBudget}
            onBlur={(e) => {
              const v = Math.max(1000, Math.round(Number(e.target.value) || 0));
              if (v !== s.tokenBudget) patch({ tokenBudget: v });
            }}
            className="h-8 w-32 rounded-md border bg-transparent px-2 text-sm tabular-nums"
          />
        </label>
        <p className="mt-1 text-xs text-muted-foreground">
          The prompt estimator warns past this; it never blocks.
        </p>

        <div className="mt-4">
          <p className="mb-1.5 text-sm text-muted-foreground">
            Excluded file extensions
          </p>
          <TokenList
            values={s.excludedExtensions}
            placeholder="e.g. lock, snap, min.js"
            transform={(v) => v.replace(/^\./, "").toLowerCase()}
            onChange={(excludedExtensions) => patch({ excludedExtensions })}
          />
          <p className="mt-1 text-xs text-muted-foreground">
            Files with these extensions are withheld from the prompt file
            picker, on top of the automatic binary and size checks.
          </p>
        </div>
      </Section>

      <Section title="Appearance">
        <ThemeToggle />
      </Section>

      <DataSection />
    </div>
  );
}

/* ----------------------------------------------------------------------- */

function ScanRootsSection() {
  const qc = useQueryClient();
  const roots = useQuery({
    queryKey: ["scanRoots"],
    queryFn: () => unwrap(commands.listScanRoots()),
  });
  const invalidate = () => qc.invalidateQueries({ queryKey: ["scanRoots"] });

  const addRoot = useMutation({
    mutationFn: async () => {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "Choose a folder to scan",
      });
      if (typeof picked !== "string") return null;
      return unwrap(commands.addScanRoot(picked));
    },
    onSuccess: invalidate,
  });
  const removeRoot = useMutation({
    mutationFn: (id: number) => unwrap(commands.removeScanRoot(id)),
    onSuccess: invalidate,
  });
  const toggle = useMutation({
    mutationFn: (v: { id: number; enabled: boolean }) =>
      unwrap(commands.setScanRootEnabled(v.id, v.enabled)),
    onSuccess: invalidate,
  });
  const reorder = useMutation({
    mutationFn: (orderedIds: number[]) =>
      unwrap(commands.reorderScanRoots(orderedIds)),
    onSuccess: invalidate,
  });

  const list = roots.data ?? [];
  const move = (index: number, dir: -1 | 1) => {
    const next = [...list];
    const j = index + dir;
    if (j < 0 || j >= next.length) return;
    [next[index], next[j]] = [next[j], next[index]];
    reorder.mutate(next.map((r) => r.id));
  };

  return (
    <section>
      <div className="mb-3 flex items-center justify-between">
        <h2 className="text-sm font-semibold">Scan roots</h2>
        <Button
          size="sm"
          onClick={() => addRoot.mutate()}
          disabled={addRoot.isPending}
        >
          <FolderPlus />
          Add folder
        </Button>
      </div>
      <div className="rounded-lg border bg-card">
        {roots.isLoading ? (
          <p className="p-4 text-sm text-muted-foreground">Loading…</p>
        ) : list.length > 0 ? (
          <ul className="divide-y">
            {list.map((root, i) => (
              <li key={root.id} className="flex items-center gap-2 p-3">
                <input
                  type="checkbox"
                  checked={root.enabled}
                  onChange={(e) =>
                    toggle.mutate({ id: root.id, enabled: e.target.checked })
                  }
                  aria-label={`${root.enabled ? "Disable" : "Enable"} ${root.path}`}
                />
                <span
                  className={
                    "flex-1 truncate font-mono text-xs" +
                    (root.enabled ? "" : " text-muted-foreground line-through")
                  }
                  title={root.path}
                >
                  {root.path}
                </span>
                <div className="flex">
                  <IconBtn
                    label="Move up"
                    disabled={i === 0 || reorder.isPending}
                    onClick={() => move(i, -1)}
                  >
                    <ChevronUp />
                  </IconBtn>
                  <IconBtn
                    label="Move down"
                    disabled={i === list.length - 1 || reorder.isPending}
                    onClick={() => move(i, 1)}
                  >
                    <ChevronDown />
                  </IconBtn>
                </div>
                <IconBtn
                  label={`Remove ${root.path}`}
                  onClick={() => removeRoot.mutate(root.id)}
                >
                  <Trash2 />
                </IconBtn>
              </li>
            ))}
          </ul>
        ) : (
          <p className="p-4 text-sm text-muted-foreground">
            No scan roots yet. Repo Radar reads git metadata and dependency
            manifests locally; nothing about your code leaves the machine.
          </p>
        )}
      </div>
    </section>
  );
}

function PruneDirs({
  extra,
  onChange,
}: {
  extra: string[];
  onChange: (v: string[]) => void;
}) {
  const builtins = useQuery({
    queryKey: ["builtinPruneDirs"],
    queryFn: () => unwrap(commands.builtinPruneDirs()),
    staleTime: Infinity,
  });

  return (
    <div>
      <p className="mb-1.5 text-sm text-muted-foreground">
        Additional prune directories
      </p>
      <TokenList
        values={extra}
        placeholder="e.g. fixtures, .cache, coverage"
        transform={(v) => v.replace(/[/\\]/g, "").trim()}
        onChange={onChange}
      />
      <p className="mt-2 text-xs text-muted-foreground">
        Always pruned (built in):{" "}
        <span className="font-mono">
          {(builtins.data ?? []).join(", ") || "…"}
        </span>
      </p>
    </div>
  );
}

function DataSection() {
  const qc = useQueryClient();
  const [confirming, setConfirming] = useState(false);

  const openFolder = useMutation({
    mutationFn: () => unwrap(commands.openDataFolder()),
  });
  const reset = useMutation({
    mutationFn: () => unwrap(commands.resetDatabase()),
    onSuccess: () => {
      setConfirming(false);
      qc.invalidateQueries();
    },
  });

  return (
    <section>
      <h2 className="mb-3 text-sm font-semibold">Data</h2>
      <div className="flex flex-wrap items-center gap-3">
        <Button
          variant="outline"
          size="sm"
          onClick={() => openFolder.mutate()}
          disabled={openFolder.isPending}
        >
          <FolderOpen />
          Open data folder
        </Button>

        {confirming ? (
          <span className="flex items-center gap-2 text-sm">
            <span className="text-muted-foreground">
              Clear all scanned repos, health data, and advisories?
            </span>
            <Button
              variant="destructive"
              size="sm"
              onClick={() => reset.mutate()}
              disabled={reset.isPending}
            >
              {reset.isPending ? "Resetting…" : "Yes, reset"}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setConfirming(false)}
              disabled={reset.isPending}
            >
              Cancel
            </Button>
          </span>
        ) : (
          <Button
            variant="outline"
            size="sm"
            onClick={() => setConfirming(true)}
          >
            <Trash2 />
            Reset database
          </Button>
        )}
      </div>
      <p className="mt-2 text-xs text-muted-foreground">
        Reset keeps your scan roots and preferences. Everything it clears is
        rebuilt by re-scanning and re-syncing.
      </p>
      {openFolder.isError ? (
        <p className="mt-1 text-xs text-vulnerability">
          Could not open the data folder.
        </p>
      ) : null}
    </section>
  );
}

/* ---- small shared bits ------------------------------------------------- */

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section>
      <h2 className="mb-3 text-sm font-semibold">{title}</h2>
      {children}
    </section>
  );
}

function IconBtn({
  label,
  onClick,
  disabled,
  children,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <Button
      size="icon"
      variant="ghost"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={label}
    >
      {children}
    </Button>
  );
}

/** Editable list of short string tokens rendered as removable chips. */
function TokenList({
  values,
  placeholder,
  transform = (v) => v.trim(),
  onChange,
}: {
  values: string[];
  placeholder?: string;
  transform?: (v: string) => string;
  onChange: (v: string[]) => void;
}) {
  const [draft, setDraft] = useState("");

  const add = () => {
    const v = transform(draft);
    if (v && !values.includes(v)) onChange([...values, v]);
    setDraft("");
  };

  return (
    <div>
      <div className="flex gap-2">
        <input
          value={draft}
          placeholder={placeholder}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              add();
            }
          }}
          className="h-8 flex-1 rounded-md border bg-transparent px-2 text-sm"
        />
        <Button size="sm" variant="secondary" onClick={add} disabled={!draft.trim()}>
          Add
        </Button>
      </div>
      {values.length > 0 ? (
        <ul className="mt-2 flex flex-wrap gap-1.5">
          {values.map((v) => (
            <li
              key={v}
              className="flex items-center gap-1 rounded bg-secondary px-1.5 py-0.5 font-mono text-xs"
            >
              {v}
              <button
                onClick={() => onChange(values.filter((x) => x !== v))}
                aria-label={`Remove ${v}`}
                className="text-muted-foreground hover:text-foreground"
              >
                <X className="size-3" />
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
