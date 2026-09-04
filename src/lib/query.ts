import { MutationCache, QueryCache, QueryClient } from "@tanstack/react-query";

import { IpcError } from "@/lib/ipc";
import { reportFatal } from "@/lib/fatal";

/**
 * One client for the app. Tauri events invalidate query keys rather than
 * writing the cache directly (DESIGN §14.1), so a single invalidation path
 * is the only way data refreshes.
 *
 * Query keys (DESIGN §14.1):
 *   ['repos', filter] · ['repo', id] · ['syncStatus'] · ['settings']
 *   ['templates'] · ['outdated', repoId] · ['bootStatus'] · ['latestScan']
 *
 * Any command that comes back tier `"fatal"` — from a query or a mutation —
 * trips the recovery screen (DESIGN §15).
 */
function onError(error: unknown) {
  if (error instanceof IpcError && error.tier === "fatal") {
    reportFatal(error.message);
  }
}

export const queryClient = new QueryClient({
  queryCache: new QueryCache({ onError }),
  mutationCache: new MutationCache({ onError }),
  defaultOptions: {
    queries: {
      staleTime: 5_000,
      refetchOnWindowFocus: false,
      retry: 1,
    },
  },
});
