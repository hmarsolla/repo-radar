import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderPlus, Radar } from "lucide-react";

import { Button } from "@/components/ui/button";
import { commands, unwrap } from "@/lib/ipc";

/**
 * Empty state shown when no scan root is configured (PRD §6, FR-10.4). One
 * sentence on what the app does, a plain statement of the network policy,
 * and a folder picker — nothing else between a fresh install and a first
 * scan (DESIGN §14.4).
 */
export function Onboarding() {
  const qc = useQueryClient();
  const addRoot = useMutation({
    mutationFn: async () => {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "Choose a folder that contains your repositories",
      });
      if (typeof picked !== "string") return null;
      return unwrap(commands.addScanRoot(picked));
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["scanRoots"] });
    },
  });

  return (
    <div className="mx-auto flex max-w-md flex-col items-center gap-4 py-16 text-center">
      <div className="rounded-full bg-secondary p-4">
        <Radar className="size-8 text-primary" />
      </div>
      <h2 className="text-xl font-semibold">Point Repo Radar at your code</h2>
      <p className="text-sm text-muted-foreground">
        Choose a folder that holds your git repositories. Repo Radar inventories
        them, reads their git history, and checks their dependencies against the
        OSV advisory database.
      </p>
      <p className="text-xs text-muted-foreground">
        Everything runs locally. The only network traffic is downloading the OSV
        advisory database — your code and dependency list never leave this
        machine.
      </p>
      <Button onClick={() => addRoot.mutate()} disabled={addRoot.isPending}>
        <FolderPlus />
        Choose a folder
      </Button>
    </div>
  );
}

/** True once at least one scan root exists. */
export function useHasScanRoot() {
  return useQuery({
    queryKey: ["scanRoots"],
    queryFn: () => unwrap(commands.listScanRoots()),
    select: (roots) => roots.length > 0,
  });
}
