import type { Warning } from "@/bindings";

/**
 * The path a [`Warning`] is scoped to, if any. `WarningScope` serializes as
 * `"Scan"` or `{ Repo: string }` / `{ File: string }`.
 */
export function scopePath(w: Warning): string | null {
  const s = w.scope as unknown;
  if (s && typeof s === "object") {
    if ("Repo" in s) return (s as { Repo: string }).Repo;
    if ("File" in s) return (s as { File: string }).File;
  }
  return null;
}
