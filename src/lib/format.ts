/** Shared value formatting for the UI. */

const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/**
 * Human-readable byte size, base 1024.
 *
 * `null` means "never measured", which must not render as `0 B` — a repository
 * an older build scanned would otherwise look like it occupies nothing.
 */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined) return "—";
  if (bytes === 0) return "0 B";

  let value = Math.abs(bytes);
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  // Bytes and KB are never fractional; larger units get one decimal so a
  // column of sizes stays comparable at a glance.
  const digits = unit <= 1 ? 0 : value < 10 ? 1 : value < 100 ? 1 : 0;
  const sign = bytes < 0 ? "-" : "";
  return `${sign}${value.toFixed(digits)} ${UNITS[unit]}`;
}

/** Compact count, e.g. `12,480`. */
export function formatCount(n: number | null | undefined): string {
  if (n === null || n === undefined) return "—";
  return n.toLocaleString();
}

/**
 * "3 days ago" / "2 years ago" from a day count.
 *
 * Deliberately coarse: the exact hour a repository was last committed to is
 * never the question being asked on these screens.
 */
export function formatAge(days: number | null | undefined): string {
  if (days === null || days === undefined) return "never";
  if (days === 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 30) return `${days} days ago`;
  if (days < 365) {
    const months = Math.round(days / 30);
    return months === 1 ? "1 month ago" : `${months} months ago`;
  }
  const years = Math.floor(days / 365);
  const months = Math.round((days % 365) / 30);
  if (years >= 3 || months === 0) {
    return years === 1 ? "1 year ago" : `${years} years ago`;
  }
  return `${years}y ${months}m ago`;
}

/** Local date from an RFC-3339 timestamp, or `—`. */
export function formatDate(timestamp: string | null | undefined): string {
  if (!timestamp) return "—";
  const d = new Date(timestamp);
  return Number.isNaN(d.getTime()) ? "—" : d.toLocaleDateString();
}
