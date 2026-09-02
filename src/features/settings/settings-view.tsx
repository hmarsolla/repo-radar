import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderPlus, Trash2 } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { commands, unwrap } from "@/lib/ipc";
import { ThemeToggle } from "./theme-toggle";

export function SettingsView() {
  const qc = useQueryClient();

  const roots = useQuery({
    queryKey: ["scanRoots"],
    queryFn: () => unwrap(commands.listScanRoots()),
  });

  const addRoot = useMutation({
    mutationFn: async () => {
      const picked = await open({ directory: true, multiple: false, title: "Choose a folder to scan" });
      if (typeof picked !== "string") return null;
      return unwrap(commands.addScanRoot(picked));
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["scanRoots"] }),
  });

  const removeRoot = useMutation({
    mutationFn: (id: number) => unwrap(commands.removeScanRoot(id)),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["scanRoots"] }),
  });

  return (
    <div className="max-w-2xl">
      <PageHeader
        title="Settings"
        description="Scan roots, appearance, and sync preferences. Everything here is stored in your OS config directory — never in a scanned repository."
      />

      <section className="mb-8">
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-sm font-semibold">Scan roots</h2>
          <Button size="sm" onClick={() => addRoot.mutate()} disabled={addRoot.isPending}>
            <FolderPlus />
            Add folder
          </Button>
        </div>
        <div className="rounded-lg border bg-card">
          {roots.isLoading ? (
            <p className="p-4 text-sm text-muted-foreground">Loading…</p>
          ) : roots.data && roots.data.length > 0 ? (
            <ul className="divide-y">
              {roots.data.map((root) => (
                <li key={root.id} className="flex items-center justify-between gap-3 p-3">
                  <span className="truncate font-mono text-xs" title={root.path}>
                    {root.path}
                  </span>
                  <Button
                    size="icon"
                    variant="ghost"
                    onClick={() => removeRoot.mutate(root.id)}
                    aria-label={`Remove ${root.path}`}
                  >
                    <Trash2 />
                  </Button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="p-4 text-sm text-muted-foreground">
              No scan roots yet. repo-radar reads git metadata and dependency
              manifests locally; nothing about your code leaves the machine.
            </p>
          )}
        </div>
      </section>

      <section>
        <h2 className="mb-3 text-sm font-semibold">Appearance</h2>
        <ThemeToggle />
      </section>
    </div>
  );
}
