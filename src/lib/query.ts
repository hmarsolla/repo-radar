import { QueryClient } from "@tanstack/react-query";

/**
 * One client for the app. Tauri events invalidate query keys rather than
 * writing the cache directly (DESIGN §14.1), so a single invalidation path
 * is the only way data refreshes.
 *
 * Query keys (DESIGN §14.1):
 *   ['repos', filter] · ['repo', id] · ['syncStatus'] · ['settings']
 *   ['templates'] · ['outdated', repoId]
 */
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 5_000,
      refetchOnWindowFocus: false,
      retry: 1,
    },
  },
});
