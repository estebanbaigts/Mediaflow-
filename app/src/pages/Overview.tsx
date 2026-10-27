import { api } from "../api";
import type { Navigate } from "../App";
import { Badge, Banner, Button, Empty, ErrorBanner, PageHeader, Section, SpaceMeter } from "../components/ui";
import { Icon } from "../components/Icon";
import { bytes, count, isResumable, relative, sessionStateLabel } from "../format";
import { useLoad } from "../hooks";
import { useRuns } from "../store";
import type { SessionInfo } from "../types";
import { useState } from "react";
import { asUiError } from "../api";
import type { UiError } from "../types";

export function SessionRow({ s, go }: { s: SessionInfo; go: Navigate }) {
  const st = sessionStateLabel[s.state];
  return (
    <button className="row row-button" onClick={() => go({ page: "history", sessionId: s.id })}>
      <Icon name="card" className="row-icon" />
      <div className="row-main">
        <div className="row-title">
          {s.project || "Untitled"} <span className="muted">· {s.source_label}</span>
        </div>
        <div className="row-sub">
          {relative(s.created_at)} · {count(s.counts.done, "file")} copied ({bytes(s.counts.bytes_done)})
          {s.counts.skipped_duplicate > 0 && ` · ${s.counts.skipped_duplicate} already there`}
          {s.counts.failed > 0 && ` · ${s.counts.failed} failed`}
        </div>
      </div>
      <Badge tone={s.running ? "info" : st.tone}>{s.running ? "Running" : st.label}</Badge>
      <Icon name="chevron" className="row-chevron" />
    </button>
  );
}

export function OverviewPage({ go }: { go: Navigate }) {
  const { data, error, loading, reload } = useLoad(() => api.overview());
  const { runs } = useRuns();
  const [actionError, setActionError] = useState<UiError | null>(null);
  const running = Object.values(runs).filter((r) => r.progress && !r.finished);

  const resume = async (id: string) => {
    try {
      await api.resume(id);
      go({ page: "import", sessionId: id });
    } catch (e) {
      setActionError(asUiError(e));
    }
  };

  return (
    <div className="page">
      <PageHeader
        title="Overview"
        subtitle="Your imports, destinations and anything that needs attention."
        actions={
          <>
            <Button icon="refresh" onClick={() => void reload()} aria-label="Refresh" />
            <Button variant="primary" icon="import" onClick={() => go({ page: "import" })}>
              New import
            </Button>
          </>
        }
      />
      <ErrorBanner error={error ?? actionError} onDismiss={() => setActionError(null)} />

      {running.map((r) => (
        <Banner
          key={r.progress!.session_id}
          tone="info"
          title={`Import in progress — ${Math.round((r.progress!.fraction ?? 0) * 100)}%`}
          actions={<Button onClick={() => go({ page: "import", sessionId: r.progress!.session_id })}>Show</Button>}
        >
          {r.progress!.current ? r.progress!.current.rel_path : "Preparing…"}
        </Banner>
      ))}

      {data && data.attention.length > 0 && (
        <Section title="Needs attention">
          <div className="list">
            {data.attention.map((s) => (
              <div className="row" key={s.id}>
                <Icon name={s.state === "interrupted" || s.state === "failed" ? "error" : "warn"} className={`row-icon tone-${sessionStateLabel[s.state].tone}`} />
                <div className="row-main">
                  <div className="row-title">
                    {s.project || "Untitled"} · {s.source_label} <Badge tone={sessionStateLabel[s.state].tone}>{sessionStateLabel[s.state].label}</Badge>
                  </div>
                  <div className="row-sub">{s.last_error ?? `${s.counts.pending} file(s) not yet copied.`}</div>
                </div>
                <div className="row-actions">
                  <Button onClick={() => go({ page: "history", sessionId: s.id })}>Details</Button>
                  {isResumable(s.state) && (
                    <Button variant="primary" icon="play" onClick={() => void resume(s.id)}>
                      {s.state === "completed_with_errors" ? "Retry failed" : "Resume"}
                    </Button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </Section>
      )}

      <div className="grid-2">
        <Section title="Recent imports" aside={<Button variant="ghost" onClick={() => go({ page: "history" })}>All history</Button>}>
          {loading && !data ? (
            <div className="skeleton-list" aria-busy="true" aria-label="Loading" />
          ) : data && data.recent.length > 0 ? (
            <div className="list">
              {data.recent.map((s) => (
                <SessionRow key={s.id} s={s} go={go} />
              ))}
            </div>
          ) : (
            <Empty icon="import" title="No imports yet">
              Connect a card or choose a folder to start your first verified import.
            </Empty>
          )}
        </Section>

        <Section title="Destinations">
          {data && data.destinations.length > 0 ? (
            <div className="list">
              {data.destinations.map((g) => {
                const vol = data.volumes.find((v) => g.path.startsWith(v.mount_point) && v.mount_point.length > 1) ?? data.volumes.find((v) => g.path.startsWith(v.mount_point));
                return (
                  <div className="row" key={g.id}>
                    <Icon name="drive" className="row-icon" />
                    <div className="row-main">
                      <div className="row-title">
                        {g.label} {!g.available && <Badge tone="warn">Not connected</Badge>}
                      </div>
                      <div className="row-sub mono-path" title={g.path}>
                        {g.path}
                      </div>
                      {g.available && <SpaceMeter total={vol?.total_bytes ?? null} free={g.free_bytes} />}
                    </div>
                    <div className="row-meta">{g.available ? `${bytes(g.free_bytes)} free` : "—"}</div>
                  </div>
                );
              })}
            </div>
          ) : (
            <Empty icon="drive" title="No destinations yet">
              Destinations are added when you prepare an import.
            </Empty>
          )}
        </Section>
      </div>

      {data && (
        <Section title="Connected volumes">
          <div className="list">
            {data.volumes.map((v) => (
              <div className="row" key={v.mount_point}>
                <Icon name={v.removable ? "card" : "drive"} className="row-icon" />
                <div className="row-main">
                  <div className="row-title">
                    {v.name || v.mount_point} {v.removable && <Badge tone="neutral">Removable</Badge>} {v.read_only && <Badge tone="neutral">Read-only</Badge>}
                  </div>
                  <div className="row-sub">
                    {v.mount_point} · {v.file_system}
                  </div>
                  <SpaceMeter total={v.total_bytes} free={v.available_bytes} />
                </div>
                <div className="row-meta">
                  {bytes(v.available_bytes)} free of {bytes(v.total_bytes)}
                </div>
              </div>
            ))}
          </div>
        </Section>
      )}
    </div>
  );
}
