/**
 * A tiny external store for **fatal** (tier `"fatal"`) errors surfaced by a
 * command mid-session — database corruption or an unreadable schema. When
 * one fires, the whole app swaps to the recovery screen (DESIGN §15, M5-4).
 *
 * Startup failures come through `boot_status` instead; this covers the case
 * where the core was fine at launch and broke later.
 */

import { useSyncExternalStore } from "react";

export type FatalState = { message: string } | null;

let current: FatalState = null;
const listeners = new Set<() => void>();

/** Latch a fatal error. The first one wins — later ones are ignored so the
 * screen shows the original cause. */
export function reportFatal(message: string): void {
  if (current) return;
  current = { message: message || "A fatal error occurred." };
  for (const l of listeners) l();
}

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function useFatalError(): FatalState {
  return useSyncExternalStore(
    subscribe,
    () => current,
    () => null,
  );
}
