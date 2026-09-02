import { commands, events } from "@/bindings";
import type { CommandError } from "@/bindings";

export { commands, events };
export type { CommandError };

/**
 * tauri-specta wraps every command result as
 * `{ status: "ok", data } | { status: "error", error }`. `unwrap` collapses
 * that to the value or throws the typed {@link CommandError}, so callers
 * (and TanStack Query) can use normal control flow.
 */
export async function unwrap<T>(
  call: Promise<{ status: "ok"; data: T } | { status: "error"; error: CommandError }>,
): Promise<T> {
  const res = await call;
  if (res.status === "ok") return res.data;
  throw new IpcError(res.error);
}

export class IpcError extends Error {
  readonly tier: CommandError["tier"];
  constructor(public readonly detail: CommandError) {
    super(detail.message);
    this.name = "IpcError";
    this.tier = detail.tier;
  }
}
