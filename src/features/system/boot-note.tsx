import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Info, X } from "lucide-react";

import { commands, unwrap } from "@/lib/ipc";

/**
 * A one-time, dismissible banner for a non-fatal startup recovery — e.g. a
 * corrupt database file was quarantined and a fresh one created (DESIGN
 * §14.4, M5-4). Reads the cached `boot_status` query; renders nothing when
 * there is no note.
 */
export function BootNote() {
  const [dismissed, setDismissed] = useState(false);
  const boot = useQuery({
    queryKey: ["bootStatus"],
    queryFn: () => unwrap(commands.bootStatus()),
    retry: false,
    staleTime: Infinity,
    gcTime: Infinity,
  });

  const note = boot.data?.note;
  if (!note || dismissed) return null;

  return (
    <div className="flex items-start gap-2 border-b border-warn/30 bg-warn/5 px-6 py-2 text-sm text-warn">
      <Info className="mt-0.5 size-4 shrink-0" />
      <p className="flex-1">{note}</p>
      <button
        onClick={() => setDismissed(true)}
        aria-label="Dismiss"
        className="text-warn/70 hover:text-warn"
      >
        <X className="size-4" />
      </button>
    </div>
  );
}
