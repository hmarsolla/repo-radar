import { QueryClientProvider, useQuery } from "@tanstack/react-query";
import { RouterProvider } from "react-router-dom";

import { commands, unwrap, IpcError } from "@/lib/ipc";
import { queryClient } from "@/lib/query";
import { useFatalError } from "@/lib/fatal";
import { router } from "@/routes";
import { ThemeProvider } from "@/features/settings/theme-provider";
import { ScanProvider } from "@/features/scan/scan-provider";
import { FatalErrorScreen } from "@/features/system/fatal-error-screen";

export default function App() {
  return (
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        <BootGate>
          <ScanProvider>
            <RouterProvider router={router} />
          </ScanProvider>
        </BootGate>
      </QueryClientProvider>
    </ThemeProvider>
  );
}

/**
 * Gate the whole app on startup health (DESIGN §14.4, §15). A fatal
 * database error — at launch (`boot_status.ok === false`) or mid-session
 * (a command returned tier `"fatal"`) — swaps in the recovery screen.
 *
 * When the Tauri IPC bridge is absent (a plain-browser UI preview), the
 * `boot_status` call throws a non-`IpcError`; that is not a fatal state, so
 * the app renders normally.
 */
function BootGate({ children }: { children: React.ReactNode }) {
  const fatal = useFatalError();
  const boot = useQuery({
    queryKey: ["bootStatus"],
    queryFn: () => unwrap(commands.bootStatus()),
    retry: false,
    staleTime: Infinity,
    gcTime: Infinity,
  });

  if (fatal) {
    return <FatalErrorScreen message={fatal.message} />;
  }
  if (boot.data && !boot.data.ok) {
    return (
      <FatalErrorScreen
        message={boot.data.failure ?? "The database could not be opened."}
        schemaTooNew={boot.data.schemaTooNew}
      />
    );
  }
  if (
    boot.isError &&
    boot.error instanceof IpcError &&
    boot.error.tier === "fatal"
  ) {
    return <FatalErrorScreen message={boot.error.message} />;
  }

  return <>{children}</>;
}
