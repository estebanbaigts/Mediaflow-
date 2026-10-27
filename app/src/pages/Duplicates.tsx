import { useMemo, useState } from "react";
import { api } from "../api";
import type { Navigate } from "../App";
import { Badge, Empty, ErrorBanner, PageHeader, Section } from "../components/ui";
import { bytes, dateTime, matchLabel } from "../format";
import { useLoad } from "../hooks";
import type { MatchLevel, PlanNote } from "../types";

/** Duplicates found by past imports, by level of certainty. Nothing here is ever deleted. */
export function DuplicatesPage({ go }: { go: Navigate }) {
  const sessions = useLoad(() => api.sessions(200));
  const [sel, setSel] = useState<string | null>(null);
  const id = sel ?? sessions.data?.[0]?.id ?? null;
  const detail = useLoad(() => (id ? api.sessionDetail(id) : Promise.resolve(null)), [id]);

  const groups = useMemo(() => {
    const verified: { src: string; existing: string; size: number }[] = [];
    const possible: { src: string; existing: string; level: MatchLevel; size: number }[] = [];
    const inSource: { src: string; other: string; size: number }[] = [];
    const d = detail.data;
    if (d) {
      for (const o of d.ops) {
        const f = d.files[o.file_id];
        if (!f) continue;
        if (o.state === "skipped_duplicate") verified.push({ src: f.rel_path, existing: o.final_rel ?? o.action_ref ?? "", size: f.size });
        let notes: PlanNote[] = [];
        try {
          notes = JSON.parse(o.notes_json) as PlanNote[];
        } catch {
          notes = [];
        }
        for (const n of notes) {
          if (n.kind === "possible_duplicate") possible.push({ src: f.rel_path, existing: n.existing_rel, level: n.level, size: f.size });
          if (n.kind === "duplicate_in_source" && o.dest_idx === 0) inSource.push({ src: f.rel_path, other: n.other_rel, size: f.size });
        }
      }
    }
    return { verified, possible, inSource };
  }, [detail.data]);

  return (
    <div className="page">
      <PageHeader
        title="Duplicates"
        subtitle="What each import recognized as already present. MediaFlow never deletes duplicates; this page is for review only."
      />
      <ErrorBanner error={sessions.error ?? detail.error} />
      {sessions.data && sessions.data.length > 0 ? (
        <>
          <div className="field inline">
            <label htmlFor="dup-session">Import</label>
            <select id="dup-session" value={id ?? ""} onChange={(e) => setSel(e.target.value)}>
              {sessions.data.map((s) => (
                <option key={s.id} value={s.id}>
                  {dateTime(s.created_at)} — {s.project || "Untitled"} · {s.source_label}
                </option>
              ))}
            </select>
          </div>
          <Section title="Exact duplicates (content verified)" aside={<Badge tone="ok">{groups.verified.length}</Badge>}>
            {groups.verified.length === 0 ? (
              <Empty icon="duplicates" title="None" />
            ) : (
              <div className="table" role="table" aria-label="Verified duplicates">
                {groups.verified.map((g) => (
                  <div className="tr two" role="row" key={g.src + g.existing}>
                    <span role="cell" className="mono-path">{g.src}</span>
                    <span role="cell" className="plan-cell">
                      <span>identical to <span className="mono-path">{g.existing}</span></span>
                      <span className="note">{bytes(g.size)} · {matchLabel.verified_identical}</span>
                    </span>
                  </div>
                ))}
              </div>
            )}
          </Section>
          <Section title="Possible duplicates (not verified, copied anyway)" aside={<Badge tone="neutral">{groups.possible.length}</Badge>}>
            {groups.possible.length === 0 ? (
              <Empty icon="duplicates" title="None" />
            ) : (
              <div className="table" role="table" aria-label="Possible duplicates">
                {groups.possible.map((g) => (
                  <div className="tr two" role="row" key={g.src + g.existing}>
                    <span role="cell" className="mono-path">{g.src}</span>
                    <span role="cell" className="plan-cell">
                      <span>resembles <span className="mono-path">{g.existing}</span></span>
                      <span className="note">{bytes(g.size)} · {matchLabel[g.level]}</span>
                    </span>
                  </div>
                ))}
              </div>
            )}
          </Section>
          <Section title="Same content twice on the source" aside={<Badge tone="neutral">{groups.inSource.length}</Badge>}>
            {groups.inSource.length === 0 ? (
              <Empty icon="duplicates" title="None" />
            ) : (
              <div className="table" role="table" aria-label="Duplicates in source">
                {groups.inSource.map((g) => (
                  <div className="tr two" role="row" key={g.src}>
                    <span role="cell" className="mono-path">{g.src}</span>
                    <span role="cell" className="plan-cell">
                      <span>same as <span className="mono-path">{g.other}</span></span>
                      <span className="note">{bytes(g.size)}</span>
                    </span>
                  </div>
                ))}
              </div>
            )}
          </Section>
        </>
      ) : (
        !sessions.loading && (
          <Empty icon="duplicates" title="No imports yet">
            <button className="link" onClick={() => go({ page: "import" })}>Start an import</button> to see duplicates here.
          </Empty>
        )
      )}
    </div>
  );
}
