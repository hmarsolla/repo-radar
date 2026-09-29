import { commands } from "@/lib/ipc";

/**
 * Send a frontend failure to the Rust log (`logs/repo-radar.log`).
 *
 * Frontend and backend failures otherwise land in different places — the
 * backend in the log file, the frontend only in a devtools console the user
 * has no way to open in a release build. Funnelling both into the log file
 * means a bug report is one file.
 *
 * Never throws: it runs on paths that are already failing, and a reporting
 * error that replaced the original one would be worse than no report.
 */
export function reportFrontendError(
  context: string,
  error: unknown,
  extra?: string | null,
) {
  const message = error instanceof Error ? error.message : String(error);
  const stack = [error instanceof Error ? error.stack : null, extra]
    .filter(Boolean)
    .join("\n");

  // Keep the console line for `tauri dev`, where devtools is open anyway.
  console.error(`[${context}]`, error);

  try {
    void commands
      .reportFrontendError(context, message, stack || null)
      // A plain-browser preview has no Tauri IPC bridge; nothing to report to.
      .catch(() => {});
  } catch {
    /* no IPC bridge at all — the console line above is all we get */
  }
}

/**
 * Catch failures that never pass through React: an exception in an event
 * handler or a timer, and a promise rejection nobody awaited. These are the
 * ones that leave the UI wedged in a half-updated state with no error
 * anywhere, which reads to the user as a freeze.
 *
 * Installed once from `main.tsx`.
 */
export function installGlobalErrorHandlers() {
  window.addEventListener("error", (event) => {
    reportFrontendError("window.error", event.error ?? event.message);
  });
  window.addEventListener("unhandledrejection", (event) => {
    reportFrontendError("unhandledrejection", event.reason);
  });
}
