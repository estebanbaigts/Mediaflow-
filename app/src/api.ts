// Typed wrappers around the whitelisted Tauri commands. The UI never sends
// file system paths: folders are referenced by grant id only.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppInfo,
  CompareReport,
  GrantView,
  ImportFinished,
  LibraryRow,
  Overview,
  PhaseProgress,
  PlanView,
  Profile,
  Progress,
  SessionDetail,
  SessionInfo,
  UiError,
  Volume,
} from "./types";

export function asUiError(e: unknown): UiError {
  if (e && typeof e === "object" && "message" in e) {
    const o = e as { code?: unknown; message: unknown };
    return { code: typeof o.code === "string" ? o.code : "error", message: String(o.message) };
  }
  return { code: "error", message: typeof e === "string" ? e : "An unexpected error occurred." };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw asUiError(e);
  }
}

export const api = {
  appInfo: () => call<AppInfo>("app_info"),
  volumes: () => call<Volume[]>("list_volumes"),
  profiles: () => call<Profile[]>("list_profiles"),
  grants: () => call<GrantView[]>("list_grants"),
  pickFolder: (kind: "source" | "destination") => call<GrantView | null>("pick_folder", { kind }),
  revokeGrant: (id: string) => call<void>("revoke_grant", { id }),
  createPlan: (args: {
    source_grant: string;
    destination_grants: string[];
    profile_id: string;
    project: string;
    label: string | null;
    quick: boolean;
  }) => call<PlanView>("create_plan", { args }),
  discardPlan: (planId: string) => call<void>("discard_plan", { planId }),
  startImport: (planId: string) => call<string>("start_import", { planId }),
  pause: (sessionId: string) => call<void>("pause_import", { sessionId }),
  resume: (sessionId: string) => call<void>("resume_import", { sessionId }),
  cancel: (sessionId: string) => call<void>("cancel_import", { sessionId }),
  sessions: (limit?: number) => call<SessionInfo[]>("list_sessions", { limit }),
  sessionDetail: (sessionId: string) => call<SessionDetail>("session_detail", { sessionId }),
  exportReport: (sessionId: string, format: "json" | "csv") => call<string | null>("export_report", { sessionId, format }),
  compare: (args: { source_grant: string; destination_grants: string[]; verified: boolean; profile_id: string | null }) =>
    call<CompareReport>("run_compare", { args }),
  library: (query: string) => call<LibraryRow[]>("library", { query }),
  overview: () => call<Overview>("overview"),
};

export const events = {
  onImportProgress: (f: (p: Progress) => void): Promise<UnlistenFn> => listen<Progress>("import-progress", (e) => f(e.payload)),
  onImportFinished: (f: (p: ImportFinished) => void): Promise<UnlistenFn> =>
    listen<ImportFinished>("import-finished", (e) => f(e.payload)),
  onPlanProgress: (f: (p: PhaseProgress) => void): Promise<UnlistenFn> => listen<PhaseProgress>("plan-progress", (e) => f(e.payload)),
  onCompareProgress: (f: (p: PhaseProgress) => void): Promise<UnlistenFn> =>
    listen<PhaseProgress>("compare-progress", (e) => f(e.payload)),
};
