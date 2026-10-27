import { useMemo, useState } from "react";
import { api, asUiError } from "../api";
import type { Navigate } from "../App";
import { Icon } from "../components/Icon";
import { Badge, Button, Confirm, Empty, ErrorBanner, Mono, PageHeader, Section, Stat } from "../components/ui";
import { bytes, dateTime, duration, isCancellable, isResumable, opStateLabel, sessionStateLabel } from "../format";
import { useLoad } from "../hooks";
import type { OpState, UiError } from "../types";
import { SessionRow } from "./Overview";

function SessionDetailView({ id, go }: { id: string; go: Navigate }) {
  const { data, error, reload } = useLoad(() => api.sessionDetail(id), [id]);
  const [filter, setFilter] = useState<OpState | "all" | "problems">("problems");
  const [actionError, setActionError] = useState<UiError | null>(null);
  const [confirmCancel, setConfirmCancel] = useState(false);
  const rows = useMemo(() => {
    if (!data) return [];
    return data.ops.filter((o) =>
      filter === "all" ? true : filter === "problems" ? o.state === "failed" || o.state === "conflict" || o.state === "pending" : o.state === filter,
    );
  }, [data, filter]);

  const act = async (f: () => Promise<unknown>) => {
    setActionError(null);
    try {
      await f();
      await reload();
    } catch (e) {
      setActionError(asUiError(e));
    }
  };

  if (error) return <ErrorBanner error={error} />;
  if (!data) return <p className="muted">Loading…</p>;
  const s = data.info;
  const r = data.report;
  const st = sessionStateLabel[s.state];
  const problems = data.ops.filter((o) => o.state === "failed" || o.state === "conflict" || o.state === "pending").length;

  return (
    <div className="page">
      <PageHeader
        title={s.project || "Untitled import"}
        subtitle={
          <>
            {s.source_label} → {s.destinations.map((d) => d.label).join(", ")} · {dateTime(s.created_at)} · <Badge tone={data.running ? "info" : st.tone}>{data.running ? "Running" : st.label}</Badge>
          </>
        }
        actions={
          <>
            <Button icon="back" onClick={() => go({ page: "history" })}>
              History
            </Button>
            {data.running && (
              <Button variant="primary" onClick={() => go({ page: "import", sessionId: id })}>
                Show progress
              </Button>
            )}
            {!data.running && isCancellable(s.state) && (
              <Button variant="danger" onClick={() => setConfirmCancel(true)}>
                Cancel import
              </Button>
            )}
            {!data.running && isResumable(s.state) && (
              <Button
                variant="primary"
                icon="play"
                onClick={() =>
                  void act(async () => {
                    await api.resume(id);
                    go({ page: "import", sessionId: id });
                  })
                }
              >
                {s.state === "completed_with_errors" ? "Retry failed" : "Resume"}
              </Button>
            )}
          </>
        }
      />
      <ErrorBanner error={actionError} onDismiss={() => setActionError(null)} />
      {s.last_error && s.state !== "completed" && <p className="callout">{s.last_error}</p>}

      <div className="stats">
        <Stat label="files scanned" value={r.totals.files_scanned.toLocaleString()} hint={bytes(r.totals.bytes_scanned)} />
        <Stat label="copied & verified" value={r.totals.copied_and_verified.toLocaleString()} hint={bytes(r.totals.bytes_copied)} tone="ok" />
        <Stat label="duplicates skipped (verified)" value={r.totals.duplicates_skipped_verified.toLocaleString()} />
        <Stat label="conflicts" value={r.totals.conflicts_not_copied.toLocaleString()} tone={r.totals.conflicts_not_copied ? "warn" : undefined} />
        <Stat label="failed" value={r.totals.failed.toLocaleString()} tone={r.totals.failed ? "bad" : undefined} />
        <Stat label="excluded" value={r.totals.files_excluded.toLocaleString()} />
        <Stat label="active time" value={duration(r.active_seconds)} />
      </div>

      <Section
        title="Operations"
        aside={
          <div className="inline-actions">
            <Button icon="export" onClick={() => void act(() => api.exportReport(id, "json"))}>
              JSON
            </Button>
            <Button icon="export" onClick={() => void act(() => api.exportReport(id, "csv"))}>
              CSV
            </Button>
          </div>
        }
      >
        <div className="tabs" role="tablist" aria-label="Filter operations">
          {(["problems", "all", "done", "skipped_duplicate", "conflict", "failed", "cancelled"] as const).map((f) => (
            <button key={f} role="tab" aria-selected={filter === f} className={`tab ${filter === f ? "active" : ""}`} onClick={() => setFilter(f)}>
              {f === "problems" ? `Needs attention` : f === "all" ? "All" : opStateLabel[f].label}
              {f === "problems" && <span className="tab-n">{problems}</span>}
            </button>
          ))}
        </div>
        {rows.length === 0 ? (
          <Empty icon="check" title={filter === "problems" ? "Nothing needs attention" : "Nothing here"} />
        ) : (
          <div className="table" role="table" aria-label="Operations">
            <div className="tr th" role="row">
              <span role="columnheader">Source</span>
              <span role="columnheader">Size</span>
              <span role="columnheader">Result</span>
              <span role="columnheader">Destination / details</span>
            </div>
            {rows.slice(0, 2000).map((o) => {
              const f = data.files[o.file_id];
              const d = s.destinations.find((x) => x.idx === o.dest_idx);
              return (
                <div className="tr" role="row" key={`${o.file_id}-${o.dest_idx}`}>
                  <span role="cell" className="mono-path" title={f?.rel_path}>
                    {f?.rel_path}
                  </span>
                  <span role="cell" className="num">
                    {bytes(f?.size)}
                  </span>
                  <span role="cell">
                    <Badge tone={opStateLabel[o.state].tone}>{opStateLabel[o.state].label}</Badge>
                  </span>
                  <span role="cell" className="plan-cell">
                    <span className="muted">{d?.label}</span> <span className="mono-path">{o.final_rel ?? o.target_rel}</span>
                    {o.sha256 && (
                      <span className="note">
                        SHA-256 <Mono title={o.sha256}>{o.sha256.slice(0, 16)}…</Mono>
                      </span>
                    )}
                    {o.error_message && <span className="note tone-bad">{o.error_message}</span>}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </Section>

      {r.limitations.length > 0 && (
        <Section title="Limitations">
          <ul className="plain">
            {r.limitations.map((l) => (
              <li key={l}>{l}</li>
            ))}
          </ul>
        </Section>
      )}

      <Section title="Journal">
        <div className="log" role="log" aria-label="Session journal">
          {data.events.map((e, i) => (
            <div key={i} className={`log-line level-${e.level}`}>
              <span className="log-time">{dateTime(e.at)}</span>
              <Icon name={e.level === "error" ? "error" : e.level === "warning" ? "warn" : "info"} size={12} />
              <span>
                {e.message}
                {e.rel_path && <span className="mono-path"> — {e.rel_path}</span>}
              </span>
            </div>
          ))}
        </div>
      </Section>

      <Confirm
        open={confirmCancel}
        title="Cancel this import?"
        danger
        confirmLabel="Cancel import"
        body={<p>Verified copies are kept; the source is not touched. A cancelled import cannot be resumed.</p>}
        onCancel={() => setConfirmCancel(false)}
        onConfirm={() => {
          setConfirmCancel(false);
          void act(() => api.cancel(id));
        }}
      />
    </div>
  );
}

export function HistoryPage({ go, sessionId, project }: { go: Navigate; sessionId?: string; project?: string }) {
  const { data, error, loading } = useLoad(() => api.sessions(1000));
  const [q, setQ] = useState(project ?? "");
  if (sessionId) return <SessionDetailView id={sessionId} go={go} />;
  const list = (data ?? []).filter((s) => !q || `${s.project} ${s.source_label} ${s.source_root}`.toLowerCase().includes(q.toLowerCase()));
  return (
    <div className="page">
      <PageHeader title="History" subtitle="Every import, with its journal. Stored only on this computer." />
      <ErrorBanner error={error} />
      <div className="search">
        <Icon name="search" />
        <input aria-label="Filter history" placeholder="Filter by project or source" value={q} onChange={(e) => setQ(e.target.value)} />
      </div>
      {loading && !data ? (
        <div className="skeleton-list" aria-busy="true" aria-label="Loading" />
      ) : list.length === 0 ? (
        <Empty icon="history" title={q ? "No matching imports" : "No imports yet"} />
      ) : (
        <div className="list">
          {list.map((s) => (
            <SessionRow key={s.id} s={s} go={go} />
          ))}
        </div>
      )}
    </div>
  );
}
