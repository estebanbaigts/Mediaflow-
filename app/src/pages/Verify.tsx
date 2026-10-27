import { useEffect, useState } from "react";
import { api, asUiError, events } from "../api";
import { Icon } from "../components/Icon";
import { Badge, Banner, Button, Empty, ErrorBanner, PageHeader, ProgressBar, Section } from "../components/ui";
import { bytes, dateTime } from "../format";
import { useLoad } from "../hooks";
import type { CompareReport, PhaseProgress, Presence, UiError } from "../types";

function presence(p: Presence): { label: string; tone: "ok" | "warn" | "bad" | "neutral" } {
  switch (p.status) {
    case "present_verified":
      return { label: "Present · verified", tone: "ok" };
    case "present_unverified":
      return { label: "Name & size match · not verified", tone: "neutral" };
    case "modified":
      return { label: "Different content", tone: "bad" };
    case "corrupted":
      return { label: "Corrupted copy", tone: "bad" };
    case "missing":
      return { label: "Missing", tone: "bad" };
    case "undetermined":
      return { label: "Cannot tell (unreadable folder)", tone: "warn" };
    case "destination_unavailable":
      return { label: "Drive not connected", tone: "warn" };
    case "source_unreadable":
      return { label: "Source unreadable", tone: "warn" };
  }
}

export function VerifyPage() {
  const grants = useLoad(() => api.grants());
  const [source, setSource] = useState<string>("");
  const [dests, setDests] = useState<string[]>([]);
  const [verified, setVerified] = useState(true);
  const [skipSystem, setSkipSystem] = useState(true);
  const [report, setReport] = useState<CompareReport | null>(null);
  const [running, setRunning] = useState<PhaseProgress | null>(null);
  const [error, setError] = useState<UiError | null>(null);
  const [onlyProblems, setOnlyProblems] = useState(true);

  useEffect(() => {
    if (!running) return;
    let un: (() => void) | undefined;
    void events.onCompareProgress((p) => setRunning(p)).then((u) => (un = u));
    return () => un?.();
  }, [running !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  const all = grants.data ?? [];
  const run = async () => {
    setError(null);
    setReport(null);
    setRunning({ phase: "comparing", done: 0, total: 0 });
    try {
      setReport(await api.compare({ source_grant: source, destination_grants: dests, verified, profile_id: skipSystem ? "custom" : null }));
    } catch (e) {
      setError(asUiError(e));
    } finally {
      setRunning(null);
    }
  };

  const entries = report ? report.entries.filter((e) => !onlyProblems || e.per_destination.some((p) => p.status !== "present_verified")) : [];

  return (
    <div className="page">
      <PageHeader title="Verification" subtitle="Check that one or more backups hold every file of a card or folder — by content, wherever the files were filed." />
      <ErrorBanner error={error ?? grants.error} onDismiss={() => setError(null)} />
      <Section title="What to check">
        <div className="form-grid">
          <div className="field">
            <label htmlFor="v-src">Source</label>
            <select id="v-src" value={source} onChange={(e) => setSource(e.target.value)}>
              <option value="">Choose…</option>
              {all.map((g) => (
                <option key={g.id} value={g.id} disabled={!g.available}>
                  {g.label} — {g.path}
                </option>
              ))}
            </select>
          </div>
          <fieldset className="field">
            <legend>Backups to check</legend>
            {all
              .filter((g) => g.id !== source)
              .map((g) => (
                <label key={g.id} className="check">
                  <input type="checkbox" checked={dests.includes(g.id)} onChange={() => setDests((d) => (d.includes(g.id) ? d.filter((x) => x !== g.id) : [...d, g.id]))} />
                  <span>
                    {g.label} <span className="muted mono-path">{g.path}</span> {!g.available && <Badge tone="warn">Not connected</Badge>}
                  </span>
                </label>
              ))}
            {all.length < 2 && <p className="muted">Add folders from New import first.</p>}
          </fieldset>
        </div>
        <label className="check">
          <input type="checkbox" checked={verified} onChange={(e) => setVerified(e.target.checked)} />
          <span>
            Verify content (SHA-256) <span className="muted">— reads every file; the only mode that can prove a backup is complete</span>
          </span>
        </label>
        <label className="check">
          <input type="checkbox" checked={skipSystem} onChange={(e) => setSkipSystem(e.target.checked)} />
          <span>
            Ignore system files (.DS_Store, Thumbs.db…) <span className="muted">— they are listed separately</span>
          </span>
        </label>
        <div className="footer-actions">
          <Button variant="primary" icon="verify" disabled={!source || dests.length === 0 || running !== null} onClick={() => void run()}>
            Check now
          </Button>
        </div>
      </Section>

      {running && (
        <Section title="Checking">
          <ProgressBar value={running.total > 0 ? running.done / running.total : null} label="Verification progress" />
          <p className="muted">{running.total > 0 ? `${running.done.toLocaleString()} of ${running.total.toLocaleString()} files` : "Reading folders…"}</p>
        </Section>
      )}

      {report && (
        <>
          <Section title="Verdict">
            {report.destinations.map((d) => (
              <Banner key={d.root} tone={d.complete ? "ok" : !d.available ? "warn" : report.mode === "quick" ? "neutral" : "bad"} title={`${d.complete ? "Complete" : "Not proven complete"} — ${d.root}`}>
                {d.explanation}
                {d.other_files > 0 && ` ${d.other_files.toLocaleString()} other file(s) in this backup are not from this source.`}
              </Banner>
            ))}
            <p className="muted">
              {report.everywhere.toLocaleString()} of {report.source_files.toLocaleString()} files present in every backup
              {report.mode === "quick" ? " (by name and size — not verified)" : " (verified)"}.
              {report.previous_check_at && ` Previous check: ${dateTime(report.previous_check_at)}.`}
              {report.excluded.length > 0 && ` ${report.excluded.length} system file(s) ignored.`}
            </p>
          </Section>
          <Section
            title="Files"
            aside={
              <label className="check small">
                <input type="checkbox" checked={onlyProblems} onChange={(e) => setOnlyProblems(e.target.checked)} />
                <span>Only problems</span>
              </label>
            }
          >
            {entries.length === 0 ? (
              <Empty icon="check" title="No problems found" />
            ) : (
              <div className="table" role="table" aria-label="Check results">
                {entries.slice(0, 3000).map((e) => (
                  <div className="tr two" role="row" key={e.source_rel}>
                    <span role="cell" className="plan-cell">
                      <span className="mono-path">{e.source_rel}</span>
                      <span className="note">{bytes(e.size)}</span>
                    </span>
                    <span role="cell" className="plan-cell">
                      {e.per_destination.map((p, i) => {
                        const v = presence(p);
                        return (
                          <span key={i} className="plan-target">
                            {report.destinations.length > 1 && <span className="muted">[{i + 1}] </span>}
                            <Badge tone={v.tone}>{v.label}</Badge> {"rel_path" in p && <span className="mono-path">{p.rel_path}</span>}
                          </span>
                        );
                      })}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </Section>
        </>
      )}
      {!report && !running && (
        <Empty icon="verify" title="No check yet">
          <Icon name="info" size={12} /> Disconnected drives are reported as “not connected”, never as missing files.
        </Empty>
      )}
    </div>
  );
}
