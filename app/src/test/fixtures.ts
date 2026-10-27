import type { GrantView, Overview, PlanView, Profile, SessionDetail, SessionInfo } from "../types";

export const profile: Profile = {
  id: "youtube",
  name: "YouTube",
  description: "One folder per shoot day.",
  template: { folders: "{project}/{date}/{category}", file_name: "{name}" },
  exclusions: [{ pattern: "**/.DS_Store", reason: "macOS folder metadata" }],
  category_overrides: [],
  verify: "full",
  duplicates: "skip_verified",
  conflicts: "keep_both_renamed",
  preserve_mtime: true,
};

export const srcGrant: GrantView = {
  id: "g-src",
  path: "/Volumes/A7S3",
  kind: "source",
  label: "A7S3",
  created_at: "2026-01-01T00:00:00Z",
  available: true,
  free_bytes: 1e9,
};

export const dstGrant: GrantView = {
  id: "g-dst",
  path: "/Volumes/RAID/Projects",
  kind: "destination",
  label: "Projects",
  created_at: "2026-01-01T00:00:00Z",
  available: true,
  free_bytes: 5e12,
};

const file = (rel: string, size: number) => ({
  rel_path: rel,
  size,
  stamp: { size, mtime_ns: 1, inode: null },
  modified: "2026-05-01T10:00:00Z",
  created: null,
  class: { category: "video" as const, confidence: "confirmed" as const },
});

export function plan(overrides: Partial<PlanView> = {}): PlanView {
  return {
    plan_id: "plan-1",
    created_at: "2026-05-01T10:00:00Z",
    source_root: "/Volumes/A7S3",
    source_label: "A7S3",
    project: "Launch",
    profile,
    destinations: [
      { root: "/Volumes/RAID/Projects", label: "Projects", required_bytes: 3000, available_bytes: 5e12, space_ok: true, existing_files: 2, index_complete: true },
    ],
    items: [
      { source: file("C0001.MP4", 2000), category: "video", source_sha256: null, excluded: null, targets: [{ dest_index: 0, target_rel: "Launch/2026-05-01/Video/C0001.MP4", action: { kind: "copy" }, notes: [] }] },
      {
        source: file("C0002.MP4", 1000),
        category: "video",
        source_sha256: "ab".repeat(32),
        excluded: null,
        targets: [{ dest_index: 0, target_rel: "Launch/2026-05-01/Video/C0002.MP4", action: { kind: "skip_duplicate", existing_rel: "old/C0002.MP4", level: "verified_identical" }, notes: [] }],
      },
      {
        source: file("C0003.MP4", 1000),
        category: "video",
        source_sha256: null,
        excluded: null,
        targets: [{ dest_index: 0, target_rel: "Launch/2026-05-01/Video/C0003.MP4", action: { kind: "conflict", existing_rel: "Launch/2026-05-01/Video/C0003.MP4", level: "same_name_different_content" }, notes: [] }],
      },
      { source: file(".DS_Store", 10), category: "other", source_sha256: null, excluded: { pattern: "**/.DS_Store", reason: "macOS folder metadata" }, targets: [] },
    ],
    scan_issues: [],
    source_scan_complete: true,
    summary: { files: 4, bytes: 4010, to_copy: 1, bytes_to_copy: 2000, skip_duplicates: 1, conflicts: 1, excluded: 1, renamed: 0, possible_duplicates: 0, type_mismatches: 0, scan_issues: 0 },
    ...overrides,
  };
}

export function session(state: SessionInfo["state"], extra: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id: "s-1",
    state,
    project: "Launch",
    profile_name: "YouTube",
    source_root: "/Volumes/A7S3",
    source_label: "A7S3",
    destinations: [{ idx: 0, root: "/Volumes/RAID/Projects", label: "Projects" }],
    created_at: "2026-05-01T10:00:00Z",
    finished_at: null,
    last_error: state === "completed_with_errors" ? "Import finished with problems: 1 operation(s) not completed." : null,
    counts: { total: 3, pending: 0, done: 1, skipped_duplicate: 1, conflict: 1, failed: 0, cancelled: 0, bytes_done: 2000 },
    running: false,
    ...extra,
  };
}

export function detail(state: SessionInfo["state"]): SessionDetail {
  return {
    info: session(state),
    running: false,
    files: [],
    ops: [],
    events: [],
    report: {
      state,
      success: state === "completed",
      active_seconds: 3,
      started_at: null,
      finished_at: null,
      totals: { files_scanned: 4, files_excluded: 1, operations: 3, copied_and_verified: 1, duplicates_skipped_verified: 1, conflicts_not_copied: 1, failed: 0, cancelled: 0, pending: 0, bytes_scanned: 4010, bytes_copied: 2000, scan_issues: 0 },
      limitations: ["Verification limits text"],
    },
  };
}

export function overview(o: Partial<Overview> = {}): Overview {
  return { recent: [], attention: [], destinations: [dstGrant], sources: [srcGrant], volumes: [], ...o };
}
