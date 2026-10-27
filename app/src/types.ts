// Mirrors of the engine's serialized types (crates/mediaflow-core).

export type SessionState =
  | "queued"
  | "scanning"
  | "planned"
  | "copying"
  | "verifying"
  | "completed"
  | "completed_with_errors"
  | "paused"
  | "interrupted"
  | "failed"
  | "cancelled";

export type OpState =
  | "pending"
  | "copying"
  | "verifying"
  | "done"
  | "skipped_duplicate"
  | "conflict"
  | "failed"
  | "cancelled";

export type MatchLevel =
  | "verified_identical"
  | "probable_identical"
  | "same_size_only"
  | "same_name_different_content"
  | "different"
  | "undetermined";

export type Category =
  | "video"
  | "audio"
  | "photo"
  | "music"
  | "voiceover"
  | "graphics"
  | "subtitles"
  | "lut"
  | "edit_project"
  | "other";

export type TypeConfidence = "confirmed" | "extension_only" | "mismatch" | "unknown";

export interface UiError {
  code: string;
  message: string;
}

export interface AppInfo {
  version: string;
  library_path: string | null;
  network: string;
}

export interface Volume {
  name: string;
  mount_point: string;
  file_system: string;
  total_bytes: number;
  available_bytes: number;
  removable: boolean;
  read_only: boolean;
}

export interface GrantView {
  id: string;
  path: string;
  kind: "source" | "destination";
  label: string;
  created_at: string;
  available: boolean;
  free_bytes: number | null;
}

export interface Template {
  folders: string;
  file_name: string;
}

export interface Profile {
  id: string;
  name: string;
  description: string;
  template: Template;
  exclusions: { pattern: string; reason: string }[];
  category_overrides: { pattern: string; category: Category }[];
  verify: "full" | "paranoid";
  duplicates: "skip_verified" | "copy_anyway";
  conflicts: "keep_both_renamed" | "flag";
  preserve_mtime: boolean;
}

export interface FileStamp {
  size: number;
  mtime_ns: number;
  inode: number | null;
}

export interface ScannedFile {
  rel_path: string;
  size: number;
  stamp: FileStamp;
  modified: string | null;
  created: string | null;
  class: { category: Category; confidence: TypeConfidence };
}

export type PlanAction =
  | { kind: "copy" }
  | { kind: "skip_duplicate"; existing_rel: string; level: MatchLevel }
  | { kind: "conflict"; existing_rel: string; level: MatchLevel };

export type PlanNote =
  | { kind: "renamed_existing_different"; original_rel: string }
  | { kind: "renamed_plan_collision"; original_rel: string }
  | { kind: "possible_duplicate"; existing_rel: string; level: MatchLevel }
  | { kind: "duplicate_in_source"; other_rel: string }
  | { kind: "type_mismatch" };

export interface PlanTarget {
  dest_index: number;
  target_rel: string;
  action: PlanAction;
  notes: PlanNote[];
}

export interface PlanItem {
  source: ScannedFile;
  category: Category;
  source_sha256: string | null;
  excluded: { pattern: string; reason: string } | null;
  targets: PlanTarget[];
}

export interface PlanDestination {
  root: string;
  label: string;
  required_bytes: number;
  available_bytes: number | null;
  space_ok: boolean;
  existing_files: number;
  index_complete: boolean;
}

export interface PlanSummary {
  files: number;
  bytes: number;
  to_copy: number;
  bytes_to_copy: number;
  skip_duplicates: number;
  conflicts: number;
  excluded: number;
  renamed: number;
  possible_duplicates: number;
  type_mismatches: number;
  scan_issues: number;
}

export interface ScanIssue {
  rel_path: string;
  kind: "symlink_skipped" | "special_file_skipped" | "unreadable";
  message: string;
}

export interface PlanView {
  plan_id: string;
  created_at: string;
  source_root: string;
  source_label: string;
  project: string;
  profile: Profile;
  destinations: PlanDestination[];
  items: PlanItem[];
  scan_issues: ScanIssue[];
  source_scan_complete: boolean;
  summary: PlanSummary;
}

export interface DestRow {
  idx: number;
  root: string;
  label: string;
}

export interface OpCounts {
  total: number;
  pending: number;
  done: number;
  skipped_duplicate: number;
  conflict: number;
  failed: number;
  cancelled: number;
  bytes_done: number;
}

export interface SessionInfo {
  id: string;
  state: SessionState;
  project: string;
  profile_name: string;
  source_root: string;
  source_label: string;
  destinations: DestRow[];
  created_at: string;
  finished_at: string | null;
  last_error: string | null;
  counts: OpCounts;
  running: boolean;
}

export interface FileRow {
  file_id: number;
  rel_path: string;
  size: number;
  category: string;
  confidence: string;
  excluded_pattern: string | null;
  excluded_reason: string | null;
  source_sha256: string | null;
}

export interface OpRow {
  file_id: number;
  dest_idx: number;
  target_rel: string;
  final_rel: string | null;
  action: "copy" | "skip_duplicate" | "conflict";
  action_ref: string | null;
  action_level: MatchLevel | null;
  state: OpState;
  sha256: string | null;
  bytes: number;
  error_code: string | null;
  error_message: string | null;
  notes_json: string;
}

export interface EventRow {
  at: string;
  level: "info" | "warning" | "error";
  code: string;
  message: string;
  rel_path: string | null;
}

export interface SessionReport {
  state: SessionState;
  success: boolean;
  active_seconds: number;
  started_at: string | null;
  finished_at: string | null;
  totals: {
    files_scanned: number;
    files_excluded: number;
    operations: number;
    copied_and_verified: number;
    duplicates_skipped_verified: number;
    conflicts_not_copied: number;
    failed: number;
    cancelled: number;
    pending: number;
    bytes_scanned: number;
    bytes_copied: number;
    scan_issues: number;
  };
  limitations: string[];
}

export interface SessionDetail {
  info: SessionInfo;
  running: boolean;
  files: FileRow[];
  ops: OpRow[];
  events: EventRow[];
  report: SessionReport;
}

export type FilePhase = "checking" | "copying" | "verifying" | "finalizing";

export interface Progress {
  session_id: string;
  state: SessionState | null;
  ops_total: number;
  ops_done: number;
  ops_failed: number;
  ops_skipped: number;
  bytes_to_copy: number;
  bytes_copied: number;
  bytes_to_verify: number;
  bytes_verified: number;
  fraction: number;
  bytes_per_second: number | null;
  eta_seconds: number | null;
  current: { file_id: number; rel_path: string; size: number; phase: FilePhase; done: number } | null;
  last_outcome: {
    file_id: number;
    dest_idx: number;
    rel_path: string;
    state: OpState;
    error_code: string | null;
    message: string | null;
  } | null;
}

export interface ImportFinished {
  session_id: string;
  state: SessionState;
  error?: UiError;
}

export interface LibraryRow {
  dest_root: string;
  rel_path: string;
  sha256: string;
  size: number;
  category: string;
  project: string;
  source_label: string;
  source_rel: string;
  session_id: string;
  verified_at: string;
}

export type Presence =
  | { status: "present_verified"; rel_path: string }
  | { status: "present_unverified"; rel_path: string }
  | { status: "modified"; rel_path: string }
  | { status: "corrupted"; rel_path: string }
  | { status: "missing" }
  | { status: "undetermined" }
  | { status: "destination_unavailable" }
  | { status: "source_unreadable"; message: string };

export interface DestinationVerdict {
  root: string;
  available: boolean;
  index_complete: boolean;
  present_verified: number;
  present_unverified: number;
  modified: number;
  corrupted: number;
  missing: number;
  undetermined: number;
  other_files: number;
  complete: boolean;
  explanation: string;
}

export interface CompareReport {
  id: string;
  at: string;
  mode: "quick" | "verified";
  source_root: string;
  source_complete: boolean;
  source_files: number;
  entries: { source_rel: string; size: number; per_destination: Presence[] }[];
  destinations: DestinationVerdict[];
  everywhere: number;
  excluded: string[];
  previous_check_at: string | null;
}

export interface Overview {
  recent: SessionInfo[];
  attention: SessionInfo[];
  destinations: GrantView[];
  sources: GrantView[];
  volumes: Volume[];
}

export interface PhaseProgress {
  phase: string;
  done: number;
  total: number;
}

export interface DetectedVolume {
  id: string;
  name: string;
  mount_point: string;
  total_bytes: number;
}
