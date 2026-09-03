import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "react-router-dom";

import { queryClient } from "@/lib/query";
import { router } from "@/routes";
import { ThemeProvider } from "@/features/settings/theme-provider";
import { ScanProvider } from "@/features/scan/scan-provider";

export default function App() {
  return (
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        <ScanProvider>
          <RouterProvider router={router} />
        </ScanProvider>
      </QueryClientProvider>
    </ThemeProvider>
  );
}
