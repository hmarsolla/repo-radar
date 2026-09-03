import { Loader2, TriangleAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useScan } from "@/features/scan/scan-provider";

/**
 * Persistent slot for scan progress, visible from every route (DESIGN
 * §14.1, M1-7). Renders nothing when idle; a progress readout and a Cancel
 * control while a scan runs.
 */
export function ScanProgressIndicator() {
  const scan = useScan();
  if (!scan.running) return null;

  const total = scan.discovered || 0;
  const done = scan.completed || 0;
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;

  return (
    <div className="flex items-center gap-3 text-sm">
      <Loader2 className="size-4 animate-spin text-primary" />
      <span className="tabular-nums text-muted-foreground">
        {total > 0 ? `${done}/${total} repos` : "discovering…"}
      </span>
      <div className="h-1.5 w-32 overflow-hidden rounded-full bg-secondary">
        <div
          className="h-full bg-primary transition-[width] duration-300"
          style={{ width: `${pct}%` }}
        />
      </div>
      {scan.warnings.length > 0 ? (
        <span className="inline-flex items-center gap-1 text-xs text-warn">
          <TriangleAlert className="size-3.5" />
          {scan.warnings.length}
        </span>
      ) : null}
      <Button size="sm" variant="ghost" onClick={() => scan.cancel()}>
        Cancel
      </Button>
    </div>
  );
}
