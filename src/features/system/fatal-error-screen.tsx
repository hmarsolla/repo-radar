import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { AlertOctagon, FolderOpen, RotateCcw, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { commands, unwrap } from "@/lib/ipc";

/**
 * The recovery screen (DESIGN §15, §14.4). Shown when startup failed
 * (`boot_status.ok === false`) or a command returned tier `"fatal"`
 * mid-session. Everything repo-radar stores is derived, so **Reset
 * database** is a safe last resort — except when a *newer* build wrote the
 * schema, where it would discard that build's data and the screen leads
 * with **Open data folder** instead.
 */
export function FatalErrorScreen({
  message,
  schemaTooNew = false,
}: {
  message: string;
  schemaTooNew?: boolean;
}) {
  const [confirming, setConfirming] = useState(false);
  const [done, setDone] = useState<null | "reset">(null);

  const openFolder = useMutation({
    mutationFn: () => unwrap(commands.openDataFolder()),
  });
  const reset = useMutation({
    mutationFn: () => unwrap(commands.resetDatabase()),
    onSuccess: () => setDone("reset"),
  });

  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-6">
      <div className="w-full max-w-lg rounded-xl border border-compromise/40 bg-card p-6">
        <div className="flex items-center gap-2 text-compromise">
          <AlertOctagon className="size-5" />
          <h1 className="text-lg font-semibold">Repo Radar can’t start</h1>
        </div>

        <p className="mt-3 text-sm text-muted-foreground">
          The local database could not be opened:
        </p>
        <pre className="mt-1 overflow-x-auto rounded-md bg-secondary p-2 text-xs">
          {message}
        </pre>

        {schemaTooNew ? (
          <p className="mt-3 rounded-md border border-warn/40 bg-warn/5 p-2 text-xs text-warn">
            This database was written by a newer version of Repo Radar.
            Resetting would discard that data. Install the newer version, or
            open the data folder to back up <code>repo-radar.db</code> first.
          </p>
        ) : (
          <p className="mt-3 text-sm text-muted-foreground">
            Everything Repo Radar stores is rebuilt by re-scanning your
            repositories and re-syncing advisories, so resetting the database
            loses no original data.
          </p>
        )}

        {done === "reset" ? (
          <p className="mt-4 rounded-md border border-ok/40 bg-ok/5 p-3 text-sm text-ok">
            Database cleared. Restart Repo Radar to continue.
          </p>
        ) : (
          <div className="mt-5 flex flex-wrap items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => openFolder.mutate()}
              disabled={openFolder.isPending}
            >
              <FolderOpen />
              Open data folder
            </Button>

            <Button
              variant="outline"
              size="sm"
              onClick={() => window.location.reload()}
            >
              <RotateCcw />
              Try again
            </Button>

            {confirming ? (
              <span className="flex items-center gap-2 text-sm">
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={() => reset.mutate()}
                  disabled={reset.isPending}
                >
                  {reset.isPending ? "Resetting…" : "Confirm reset"}
                </Button>
                <button
                  className="text-xs text-muted-foreground underline"
                  onClick={() => setConfirming(false)}
                >
                  cancel
                </button>
              </span>
            ) : (
              <Button
                variant={schemaTooNew ? "ghost" : "destructive"}
                size="sm"
                onClick={() => setConfirming(true)}
              >
                <Trash2 />
                Reset database
              </Button>
            )}
          </div>
        )}

        {reset.isError ? (
          <p className="mt-2 text-xs text-compromise">
            Reset failed. Close Repo Radar and delete{" "}
            <code>repo-radar.db</code> from the data folder by hand.
          </p>
        ) : null}
      </div>
    </div>
  );
}
