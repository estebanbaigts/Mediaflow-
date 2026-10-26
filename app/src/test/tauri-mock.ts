// In-memory stand-in for the Tauri bridge used by UI tests.
import { vi } from "vitest";

type Handler = (args: Record<string, unknown> | undefined) => unknown;
const handlers = new Map<string, Handler>();
const listeners = new Map<string, Set<(e: { payload: unknown }) => void>>();
export const calls: { cmd: string; args: Record<string, unknown> | undefined }[] = [];

export function onInvoke(cmd: string, h: Handler) {
  handlers.set(cmd, h);
}

export function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((l) => l({ payload }));
}

export function resetMock() {
  handlers.clear();
  listeners.clear();
  calls.length = 0;
}

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args });
    const h = handlers.get(cmd);
    if (!h) throw { code: "not_mocked", message: `command ${cmd} not mocked` };
    return h(args);
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, cb: (e: { payload: unknown }) => void) => {
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event)!.add(cb);
    return () => listeners.get(event)?.delete(cb);
  }),
}));
