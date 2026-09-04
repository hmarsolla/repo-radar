import { NavLink, Outlet } from "react-router-dom";
import { Radar } from "lucide-react";

import { cn } from "@/lib/utils";
import { NAV_ITEMS } from "@/lib/nav";
import { ScanProgressIndicator } from "./scan-progress-indicator";
import { FreshnessIndicator } from "./freshness-indicator";
import { BootNote } from "@/features/system/boot-note";

export function AppLayout() {
  return (
    <div className="grid h-full grid-rows-[auto_1fr] md:grid-cols-[15rem_1fr] md:grid-rows-none">
      {/* Sidebar */}
      <aside className="flex flex-col gap-1 border-b bg-card px-3 py-3 md:border-b-0 md:border-r md:py-5">
        <div className="mb-3 flex items-center gap-2 px-2 font-semibold">
          <Radar className="size-5 text-primary" />
          <span>repo-radar</span>
        </div>
        <nav className="flex flex-row gap-1 md:flex-col">
          {NAV_ITEMS.map(({ to, label, icon: Icon, end }) => (
            <NavLink
              key={to}
              to={to}
              end={end}
              className={({ isActive }) =>
                cn(
                  "flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                  isActive
                    ? "bg-secondary text-secondary-foreground"
                    : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                )
              }
            >
              <Icon className="size-4" />
              <span className="hidden sm:inline">{label}</span>
            </NavLink>
          ))}
        </nav>
      </aside>

      {/* Main column */}
      <div className="flex min-h-0 flex-col">
        <BootNote />
        <header className="flex items-center justify-between gap-4 border-b px-6 py-3">
          <ScanProgressIndicator />
          <div className="ml-auto">
            <FreshnessIndicator />
          </div>
        </header>
        <main className="min-h-0 flex-1 overflow-auto p-6">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
