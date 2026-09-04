import { createBrowserRouter } from "react-router-dom";

import { AppLayout } from "@/components/layout/app-layout";
import { DashboardView } from "@/features/dashboard/dashboard-view";
import { ReposView } from "@/features/repos/repos-view";
import { RepoDetailView } from "@/features/repos/repo-detail-view";
import { AdvisoriesView } from "@/features/advisories/advisories-view";
import { PromptsView } from "@/features/prompts/prompts-view";
import { SettingsView } from "@/features/settings/settings-view";

export const router = createBrowserRouter([
  {
    path: "/",
    element: <AppLayout />,
    children: [
      { index: true, element: <DashboardView /> },
      { path: "repos", element: <ReposView /> },
      { path: "repos/:id", element: <RepoDetailView /> },
      { path: "advisories", element: <AdvisoriesView /> },
      { path: "prompts", element: <PromptsView /> },
      { path: "settings", element: <SettingsView /> },
    ],
  },
]);
