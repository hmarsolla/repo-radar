import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useQueryClient } from "@tanstack/react-query";

import { commands, events, unwrap } from "@/lib/ipc";
import type { Warning } from "@/bindings";

type ScanState = {
  running: boolean;
  scanId: number | null;
  discovered: number;
  completed: number;
  /** Warnings seen during the current/last scan. */
  warnings: Warning[];
  error: string | null;
};

type ScanContextValue = ScanState & {
  start: () => Promise<void>;
  cancel: () => Promise<void>;
};

const initial: ScanState = {
  running: false,
  scanId: null,
  discovered: 0,
  completed: 0,
  warnings: [],
  error: null,
};

const ScanCtx = createContext<ScanContextValue | null>(null);

export function ScanProvider({ children }: { children: React.ReactNode }) {
  const qc = useQueryClient();
  const [state, setState] = useState<ScanState>(initial);
  const scanIdRef = useRef<number | null>(null);

  useEffect(() => {
    const unlisten: Array<() => void> = [];
    let disposed = false;

    // `listen` throws synchronously when the Tauri IPC bridge is absent
    // (e.g. the app opened in a plain browser for a UI preview). Swallow
    // that so the rest of the app still renders; events simply won't fire.
    const track = (make: () => Promise<() => void>) => {
      try {
        make().then((fn) => {
          if (disposed) fn();
          else unlisten.push(fn);
        });
      } catch {
        /* not running inside Tauri */
      }
    };

    track(() =>
      events.scanProgress.listen((e) => {
        if (e.payload.scanId !== scanIdRef.current) return;
        setState((s) => ({
          ...s,
          discovered: e.payload.discovered,
          completed: e.payload.completed,
        }));
      }),
    );
    track(() =>
      events.scanRepoDone.listen((e) => {
        if (e.payload.scanId !== scanIdRef.current) return;
        qc.invalidateQueries({ queryKey: ["repos"] });
        qc.invalidateQueries({ queryKey: ["repo", e.payload.repoId] });
        qc.invalidateQueries({ queryKey: ["dashboard"] });
      }),
    );
    track(() =>
      events.scanWarning.listen((e) => {
        if (e.payload.scanId !== scanIdRef.current) return;
        setState((s) => ({ ...s, warnings: [...s.warnings, e.payload.warning] }));
      }),
    );
    track(() =>
      events.scanComplete.listen((e) => {
        if (e.payload.scanId !== scanIdRef.current) return;
        scanIdRef.current = null;
        setState((s) => ({ ...s, running: false }));
        qc.invalidateQueries({ queryKey: ["repos"] });
        qc.invalidateQueries({ queryKey: ["dashboard"] });
      }),
    );
    track(() =>
      events.scanError.listen((e) => {
        scanIdRef.current = null;
        setState((s) => ({ ...s, running: false, error: e.payload.message }));
      }),
    );

    return () => {
      disposed = true;
      unlisten.forEach((fn) => fn());
    };
  }, [qc]);

  const start = useCallback(async () => {
    setState({ ...initial, running: true });
    try {
      const id = await unwrap(commands.scanStart());
      scanIdRef.current = id;
      setState((s) => ({ ...s, scanId: id }));
    } catch (err) {
      scanIdRef.current = null;
      setState((s) => ({
        ...s,
        running: false,
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  }, []);

  const cancel = useCallback(async () => {
    const id = scanIdRef.current;
    if (id == null) return;
    await unwrap(commands.scanCancel(id));
  }, []);

  const value = useMemo(
    () => ({ ...state, start, cancel }),
    [state, start, cancel],
  );

  return <ScanCtx.Provider value={value}>{children}</ScanCtx.Provider>;
}

export function useScan() {
  const ctx = useContext(ScanCtx);
  if (!ctx) throw new Error("useScan must be used within <ScanProvider>");
  return ctx;
}
