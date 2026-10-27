import type { Category, MatchLevel, OpState, SessionState } from "./types";

export function bytes(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n < 1000) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB", "PB"];
  let v = n;
  let i = -1;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}

export function count(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

export function duration(secs: number | null | undefined): string {
  if (secs == null || !isFinite(secs)) return "—";
  const s = Math.max(0, Math.round(secs));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ${String(s % 60).padStart(2, "0")} s`;
  return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")} min`;
}

export function dateTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return "—";
  return d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

export function relative(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso).getTime();
  const diff = (Date.now() - d) / 1000;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} h ago`;
  return dateTime(iso);
}

export type Tone = "neutral" | "ok" | "warn" | "bad" | "info";

export const sessionStateLabel: Record<SessionState, { label: string; tone: Tone }> = {
  queued: { label: "Queued", tone: "neutral" },
  scanning: { label: "Scanning", tone: "info" },
  planned: { label: "Planned, not started", tone: "neutral" },
  copying: { label: "Copying", tone: "info" },
  verifying: { label: "Verifying", tone: "info" },
  completed: { label: "Completed · verified", tone: "ok" },
  completed_with_errors: { label: "Finished with problems", tone: "warn" },
  paused: { label: "Paused", tone: "neutral" },
  interrupted: { label: "Interrupted", tone: "bad" },
  failed: { label: "Failed", tone: "bad" },
  cancelled: { label: "Cancelled", tone: "neutral" },
};

export const opStateLabel: Record<OpState, { label: string; tone: Tone }> = {
  pending: { label: "Pending", tone: "neutral" },
  copying: { label: "Copying", tone: "info" },
  verifying: { label: "Verifying", tone: "info" },
  done: { label: "Copied · verified", tone: "ok" },
  skipped_duplicate: { label: "Already there · verified", tone: "ok" },
  conflict: { label: "Conflict · not copied", tone: "warn" },
  failed: { label: "Failed", tone: "bad" },
  cancelled: { label: "Cancelled", tone: "neutral" },
};

export const matchLabel: Record<MatchLevel, string> = {
  verified_identical: "Identical content (SHA-256 verified)",
  probable_identical: "Probably identical — not verified yet",
  same_size_only: "Same size only — not verified",
  same_name_different_content: "Same name, different content",
  different: "Different",
  undetermined: "Cannot be determined",
};

export const categoryLabel: Record<Category, string> = {
  video: "Video",
  audio: "Audio",
  photo: "Photo",
  music: "Music",
  voiceover: "Voice-over",
  graphics: "Graphics",
  subtitles: "Subtitles",
  lut: "LUT / color",
  edit_project: "Edit project",
  other: "Other",
};

export function isResumable(s: SessionState): boolean {
  return s === "paused" || s === "interrupted" || s === "failed" || s === "completed_with_errors";
}

export function isCancellable(s: SessionState): boolean {
  return s === "planned" || s === "paused" || s === "interrupted" || s === "failed";
}
