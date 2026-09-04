import {
  LayoutDashboard,
  FolderGit2,
  ShieldAlert,
  Sparkles,
  Settings,
  type LucideIcon,
} from "lucide-react";

export type NavItem = {
  to: string;
  label: string;
  icon: LucideIcon;
  /** Match child routes too (e.g. /repos/:id under "Repositories"). */
  end?: boolean;
};

export const NAV_ITEMS: NavItem[] = [
  { to: "/", label: "Dashboard", icon: LayoutDashboard, end: true },
  { to: "/repos", label: "Repositories", icon: FolderGit2 },
  { to: "/advisories", label: "Advisories", icon: ShieldAlert },
  { to: "/prompts", label: "Prompts", icon: Sparkles },
  { to: "/settings", label: "Settings", icon: Settings },
];
