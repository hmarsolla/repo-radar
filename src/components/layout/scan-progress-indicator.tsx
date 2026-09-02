/**
 * Persistent slot for scan progress, visible from every route (DESIGN
 * §14.1). Wired to `scan:progress` / `scan:complete` events and a cancel
 * control in **M1-7**. Renders nothing while idle, which is the only state
 * that exists before the scan pipeline lands.
 */
export function ScanProgressIndicator() {
  return null;
}
