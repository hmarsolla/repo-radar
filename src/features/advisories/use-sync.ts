import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, events, unwrap } from "@/lib/ipc";
import type { SyncMode } from "@/bindings";

/** Advisory-database status: freshness, per-ecosystem counts, last error. */
export function useSyncStatus() {
  const qc = useQueryClient();

  useEffect(() => {
    let disposed = false;
    const unlisten: Array<() => void> = [];
    const track = (make: () => Promise<() => void>) => {
      try {
        make().then((fn) => (disposed ? fn() : unlisten.push(fn)));
      } catch {
        /* not in Tauri */
      }
    };
    track(() =>
      events.syncProgress.listen(() => {
        qc.invalidateQueries({ queryKey: ["syncStatus"] });
      }),
    );
    track(() =>
      events.syncComplete.listen(() => {
        qc.invalidateQueries({ queryKey: ["syncStatus"] });
        qc.invalidateQueries({ queryKey: ["repos"] });
        qc.invalidateQueries({ queryKey: ["repo"] });
        qc.invalidateQueries({ queryKey: ["dashboard"] });
      }),
    );
    return () => {
      disposed = true;
      unlisten.forEach((fn) => fn());
    };
  }, [qc]);

  return useQuery({
    queryKey: ["syncStatus"],
    queryFn: () => unwrap(commands.getSyncStatus()),
    refetchInterval: 60_000,
  });
}

export function useSyncNow() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (mode: SyncMode) => unwrap(commands.syncAdvisories(mode)),
    onSettled: () => qc.invalidateQueries({ queryKey: ["syncStatus"] }),
  });
}
