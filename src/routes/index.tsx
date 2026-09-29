import { createBrowserRouter } from "react-router-dom";
import type { ReactElement } from "react";

import { AppLayout } from "@/components/layout/app-layout";
import { ErrorBoundary } from "@/features/system/error-boundary";
import { DashboardView } from "@/features/dashboard/dashboard-view";
import { ReposView } from "@/features/repos/repos-view";
import { RepoDetailView } from "@/features/repos/repo-detail-view";
import { CleanupView } from "@/features/cleanup/cleanup-view";
import { AdvisoriesView } from "@/features/advisories/advisories-view";
import { PromptsView } from "@/features/prompts/prompts-view";
import { SettingsView } from "@/features/settings/settings-view";

/**
 * Give each route its own error boundary, so one view that throws shows an
 * inline notice inside the layout instead of blanking the window — the
 * navigation survives and the user can move to a working screen.
 */
function guard(context: string, element: ReactElement) {
  return (
    <ErrorBoundary context={context} inline>
      {element}
    </ErrorBoundary>
  );
}

export const router = createBrowserRouter([
  {
    path: "/",
    element: <AppLayout />,
    children: [
      { index: true, element: guard("dashboard", <DashboardView />) },
      { path: "repos", element: guard("repos", <ReposView />) },
      { path: "repos/:id", element: guard("repo-detail", <RepoDetailView />) },
      { path: "cleanup", element: guard("cleanup", <CleanupView />) },
      { path: "advisories", element: guard("advisories", <AdvisoriesView />) },
      { path: "prompts", element: guard("prompts", <PromptsView />) },
      { path: "settings", element: guard("settings", <SettingsView />) },
    ],
  },
]);
