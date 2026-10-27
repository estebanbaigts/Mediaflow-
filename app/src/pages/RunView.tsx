import { useEffect, useState } from "react";
import { api, asUiError } from "../api";
import type { Navigate } from "../App";
import { Icon } from "../components/Icon";
import { Badge, Banner, Button, Confirm, ErrorBanner, ProgressBar, Section, Stat } from "../components/ui";
import { bytes, count, duration, isCancellable, isResumable, opStateLabel, sessionStateLabel } from "../format";
import { useRun } from "../store";
import type { SessionDetail, UiError } from "../types";

const phaseLabel = { checking: "Checking", copying: "Copying", verifying: "Verifying", finalizing: "Finalizing" } as const;

/** Live view of one import. Every number comes from engine events or the journal. */
export function RunView({ sessionId, go }: { sessionId: string; go: Navigate }) {
  const run = useRun(sessionId);
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [error, setError] = useState<UiError | null>(null);
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [pausing, setPausing] = useState(false);

  const refresh = async () => {
    try {
      setDetail(await api.sessionDetail(sessionId));
    } catch (e) {
      setError(asUiError(e));
    }
  };

  // Load on mount and whenever a run finishes.
  useEffect(() => {
    void refresh();
    setPausing(false);
  }, [sessionId, run?.finished]); // eslint-disable-line react-hooks/exhaustive-deps

  const p = run?.progress ?? null;
  const live = !!p && !run?.finished;
  const state = live ? (p?.state ?? "copying") : (detail?.info.state ?? run?.finished?.state ?? null);

  const act = async (f: () => Promise<void>) => {
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(asUiError(e));
    }
  };

  const verdict = !live && detail ? detail.info : null;
  const errors = (run?.outcomes ?? []).filter((o) => o.state === "failed" || o.state === "conflict");

  return (
    <div className="run">
      <ErrorBanner error={error ?? run?.finished?.error ?? null} onDismiss={() => setError(null)} />

      {live && p && (
        <Section
          title={state === "verifying" ? "Final verification" : "Copying and verifying"}
          aside={<Badge tone="info">{sessionStateLabel[state ?? "copying"].label}</Badge>}
        >
          <div className="run-head">
            <div className="run-pct" aria-live="off">
              {Math.floor(p.fraction * 100)}
              <span>%</span>
            </div>
            <div className="run-figures">
              <div>
                {bytes(p.bytes_copied)} of {bytes(p.bytes_to_copy)} copied · {bytes(p.bytes_verified)} of {bytes(p.bytes_to_verify)} re-read
              </div>
              <div className="muted">
                {p.bytes_per_second ? `${bytes(p.bytes_per_second)}/s` : "Measuring speed…"}
                {p.eta_seconds != null && ` · about ${duration(p.eta_seconds)} left`}
              </div>
            </div>
          </div>
          <ProgressBar value={p.fraction} label="Overall progress" />
          {p.current && (
            <div className="current">
              <span className="current-phase">{phaseLabel[p.current.phase]}</span>
              <span className="mono-path" title={p.current.rel_path}>
                {p.current.rel_path}
              </span>
              <span className="muted">{bytes(p.current.size)}</span>
              <ProgressBar value={p.current.size > 0 ? p.current.done / p.current.size : null} label="Current file" />
            </div>
          )}
          <div className="stats compact">
            <Stat label="done" value={(p.ops_done + p.ops_skipped).toLocaleString()} hint={`of ${p.ops_total.toLocaleString()}`} tone="ok" />
            <Stat label="failed" value={p.ops_failed.toLocaleString()} tone={p.ops_failed ? "bad" : undefined} />
          </div>
          <div className="footer-actions">
            <Button
              icon="pause"
              disabled={pausing}
              onClick={() =>
                void act(async () => {
                  setPausing(true);
                  await api.pause(sessionId);
                })
              }
            >
              {pausing ? "Pausing after this block…" : "Pause"}
            </Button>
            <Button variant="danger" icon="stop" onClick={() => setConfirmCancel(true)}>
              Cancel
            </Button>
          </div>
        </Section>
      )}

      {verdict && (
        <Section title="Result">
          {verdict.state === "completed" && (
            <Banner tone="ok" title="Import complete — every file is verified">
              {count(verdict.counts.done, "file")} copied and verified by SHA-256 ({bytes(verdict.counts.bytes_done)})
              {verdict.counts.skipped_duplicate > 0 && `, ${count(verdict.counts.skipped_duplicate, "file")} already present with identical content`}. The source was not modified.
            </Banner>
          )}
          {verdict.state === "completed_with_errors" && (
            <Banner
              tone="warn"
              title="Finished with problems — do not format the card yet"
              actions={
                <Button variant="primary" icon="refresh" onClick={() => void act(() => api.resume(sessionId))}>
                  Retry failed
                </Button>
              }
            >
              {verdict.last_error}
            </Banner>
          )}
          {(verdict.state === "paused" || verdict.state === "interrupted" || verdict.state === "failed") && (
            <Banner
              tone={verdict.state === "paused" ? "neutral" : "bad"}
              title={verdict.state === "paused" ? "Import paused" : verdict.state === "interrupted" ? "Import interrupted" : "Import failed"}
              actions={
                <>
                  {isCancellable(verdict.state) && (
                    <Button variant="danger" onClick={() => setConfirmCancel(true)}>
                      Cancel import
                    </Button>
                  )}
                  {isResumable(verdict.state) && (
                    <Button variant="primary" icon="play" onClick={() => void act(() => api.resume(sessionId))}>
                      Resume
                    </Button>
                  )}
                </>
              }
            >
              {verdict.last_error ?? "Files already verified are kept. Resuming re-checks the disks first, then continues."}
            </Banner>
          )}
          {verdict.state === "cancelled" && (
            <Banner tone="neutral" title="Import cancelled">
              Files that were already copied and verified were kept. Nothing was deleted from the source.
            </Banner>
          )}
          {verdict.state === "planned" && (
            <Banner tone="neutral" title="Not started" actions={<Button variant="primary" icon="play" onClick={() => void act(() => api.resume(sessionId))}>Start</Button>} />
          )}
          <div className="stats">
            <Stat label="copied & verified" value={verdict.counts.done.toLocaleString()} tone="ok" hint={bytes(verdict.counts.bytes_done)} />
            <Stat label="already there (verified)" value={verdict.counts.skipped_duplicate.toLocaleString()} />
            <Stat label="conflicts, not copied" value={verdict.counts.conflict.toLocaleString()} tone={verdict.counts.conflict ? "warn" : undefined} />
            <Stat label="failed" value={verdict.counts.failed.toLocaleString()} tone={verdict.counts.failed ? "bad" : undefined} />
            <Stat label="not done yet" value={(verdict.counts.pending + verdict.counts.cancelled).toLocaleString()} />
          </div>
          {detail && detail.report.limitations.length > 0 && (
            <details className="disclosure">
              <summary>Limitations of this verification</summary>
              <ul className="plain">
                {detail.report.limitations.map((l) => (
                  <li key={l}>{l}</li>
                ))}
              </ul>
            </details>
          )}
          <div className="footer-actions">
            <Button icon="history" onClick={() => go({ page: "history", sessionId })}>
              Full details
            </Button>
            <Button icon="export" onClick={() => void act(async () => void (await api.exportReport(sessionId, "json")))}>
              Export JSON
            </Button>
            <Button icon="export" onClick={() => void act(async () => void (await api.exportReport(sessionId, "csv")))}>
              Export CSV
            </Button>
            <Button variant="primary" icon="import" onClick={() => go({ page: "import" })}>
              New import
            </Button>
          </div>
        </Section>
      )}

      {!live && !detail && !error && <p className="muted">Loading…</p>}

      {errors.length > 0 && (
        <Section title="Problems during this run">
          <div className="list">
            {errors.map((o) => (
              <div className="row" key={`${o.file_id}-${o.dest_idx}-${o.state}`}>
                <Icon name={o.state === "failed" ? "error" : "warn"} className={`row-icon tone-${opStateLabel[o.state].tone}`} />
                <div className="row-main">
                  <div className="row-title mono-path">{o.rel_path}</div>
                  <div className="row-sub">{o.message}</div>
                </div>
                <Badge tone={opStateLabel[o.state].tone}>{opStateLabel[o.state].label}</Badge>
              </div>
            ))}
          </div>
        </Section>
      )}

      <Confirm
        open={confirmCancel}
        title="Cancel this import?"
        danger
        confirmLabel="Cancel import"
        body={
          <p>
            Files already copied and verified stay at the destination. The file being copied right now is discarded. Nothing on the source is touched. A
            cancelled import cannot be resumed.
          </p>
        }
        onCancel={() => setConfirmCancel(false)}
        onConfirm={() => {
          setConfirmCancel(false);
          void act(async () => {
            await api.cancel(sessionId);
            if (!live) await refresh();
          });
        }}
      />
    </div>
  );
}
