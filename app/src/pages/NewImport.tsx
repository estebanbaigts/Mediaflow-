import { useEffect, useMemo, useState } from "react";
import { api, asUiError, events } from "../api";
import type { Navigate } from "../App";
import { Icon } from "../components/Icon";
import { Badge, Banner, Button, Empty, ErrorBanner, PageHeader, ProgressBar, Section, SpaceMeter, Stat } from "../components/ui";
import { bytes, categoryLabel, count, matchLabel } from "../format";
import { useLoad } from "../hooks";
import type { GrantView, PhaseProgress, PlanItem, PlanNote, PlanView, UiError } from "../types";
import { RunView } from "./RunView";

const STEPS = ["Source", "Destinations & profile", "Preview", "Import"] as const;

function Stepper({ step }: { step: number }) {
  return (
    <ol className="stepper" aria-label="Import steps">
      {STEPS.map((s, i) => (
        <li key={s} className={i === step ? "current" : i < step ? "done" : ""} aria-current={i === step ? "step" : undefined}>
          <span className="step-num">{i < step ? <Icon name="check" size={12} /> : i + 1}</span>
          <span>{s}</span>
        </li>
      ))}
    </ol>
  );
}

function FolderChoice({
  grants,
  selected,
  multiple,
  onToggle,
  onAdd,
  addLabel,
  kind,
}: {
  grants: GrantView[];
  selected: string[];
  multiple: boolean;
  onToggle: (id: string) => void;
  onAdd: () => void;
  addLabel: string;
  kind: "source" | "destination";
}) {
  return (
    <div className="choice-list" role={multiple ? "group" : "radiogroup"} aria-label={kind === "source" ? "Sources" : "Destinations"}>
      {grants.map((g) => {
        const on = selected.includes(g.id);
        return (
          <label key={g.id} className={`choice ${on ? "on" : ""} ${g.available ? "" : "unavailable"}`}>
            <input
              type={multiple ? "checkbox" : "radio"}
              name={kind}
              checked={on}
              disabled={!g.available}
              onChange={() => onToggle(g.id)}
            />
            <Icon name={kind === "source" ? "card" : "drive"} size={20} />
            <span className="choice-main">
              <span className="choice-title">
                {g.label} {!g.available && <Badge tone="warn">Not connected</Badge>}
              </span>
              <span className="choice-sub mono-path" title={g.path}>
                {g.path}
              </span>
            </span>
            {g.available && kind === "destination" && <span className="choice-meta">{bytes(g.free_bytes)} free</span>}
          </label>
        );
      })}
      <button type="button" className="choice choice-add" onClick={onAdd}>
        <Icon name="plus" size={20} />
        <span className="choice-main">
          <span className="choice-title">{addLabel}</span>
          <span className="choice-sub">Opens the system folder picker. MediaFlow can only access folders you choose.</span>
        </span>
      </button>
    </div>
  );
}

function noteText(n: PlanNote): string {
  switch (n.kind) {
    case "renamed_existing_different":
      return `Renamed: "${n.original_rel}" already holds a different file`;
    case "renamed_plan_collision":
      return `Renamed: several source files would land on "${n.original_rel}"`;
    case "possible_duplicate":
      return `Possible duplicate of "${n.existing_rel}" (${matchLabel[n.level].toLowerCase()}) — will be copied`;
    case "duplicate_in_source":
      return `Same content as "${n.other_rel}" in the source`;
    case "type_mismatch":
      return "File content does not match its extension";
  }
}

type Filter = "all" | "copy" | "skip" | "conflict" | "excluded" | "notes";

function itemKind(it: PlanItem): Exclude<Filter, "all" | "notes"> {
  if (it.excluded) return "excluded";
  if (it.targets.some((t) => t.action.kind === "copy")) return "copy";
  if (it.targets.some((t) => t.action.kind === "conflict")) return "conflict";
  return "skip";
}

function TreePreview({ plan }: { plan: PlanView }) {
  const tree = useMemo(() => {
    const m = new Map<string, { files: number; bytes: number }>();
    for (const it of plan.items) {
      const t = it.targets[0];
      if (!t || t.action.kind !== "copy") continue;
      const dir = t.target_rel.includes("/") ? t.target_rel.slice(0, t.target_rel.lastIndexOf("/")) : "(destination root)";
      const v = m.get(dir) ?? { files: 0, bytes: 0 };
      v.files++;
      v.bytes += it.source.size;
      m.set(dir, v);
    }
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [plan]);
  if (tree.length === 0) return <p className="muted">No new folders will be created.</p>;
  return (
    <ul className="tree" aria-label="Folders that will receive files">
      {tree.map(([dir, v]) => (
        <li key={dir}>
          <Icon name="folder" />
          <span className="mono-path">{dir}</span>
          <span className="muted">
            {count(v.files, "file")} · {bytes(v.bytes)}
          </span>
        </li>
      ))}
    </ul>
  );
}

function PlanPreview({ plan }: { plan: PlanView }) {
  const [filter, setFilter] = useState<Filter>("all");
  const [limit, setLimit] = useState(300);
  const s = plan.summary;
  const rows = plan.items.filter((it) => {
    if (filter === "all") return true;
    if (filter === "notes") return it.targets.some((t) => t.notes.length > 0);
    return itemKind(it) === filter;
  });
  const tabs: { id: Filter; label: string; n: number }[] = [
    { id: "all", label: "All", n: plan.items.length },
    { id: "copy", label: "To copy", n: s.to_copy },
    { id: "skip", label: "Already there", n: s.skip_duplicates },
    { id: "conflict", label: "Conflicts", n: s.conflicts },
    { id: "excluded", label: "Excluded", n: s.excluded },
    { id: "notes", label: "Flagged", n: plan.items.filter((it) => it.targets.some((t) => t.notes.length > 0)).length },
  ];
  return (
    <>
      <div className="stats">
        <Stat label="to copy" value={s.to_copy.toLocaleString()} hint={bytes(s.bytes_to_copy)} tone="info" />
        <Stat label="already there (verified)" value={s.skip_duplicates.toLocaleString()} tone={s.skip_duplicates ? "ok" : undefined} />
        <Stat label="conflicts, not copied" value={s.conflicts.toLocaleString()} tone={s.conflicts ? "warn" : undefined} />
        <Stat label="renamed to avoid overwrite" value={s.renamed.toLocaleString()} />
        <Stat label="excluded by profile" value={s.excluded.toLocaleString()} />
        <Stat label="not importable" value={s.scan_issues.toLocaleString()} tone={s.scan_issues ? "warn" : undefined} hint={s.scan_issues ? "links, special or unreadable" : undefined} />
      </div>

      {!plan.source_scan_complete && (
        <Banner tone="warn" title="Some source folders could not be read">
          The inventory is incomplete. Files in those folders will not be imported.
        </Banner>
      )}
      {s.possible_duplicates > 0 && (
        <Banner tone="info" title={`${count(s.possible_duplicates, "possible duplicate")} not verified`}>
          These files resemble files already at the destination but their content was not proven identical, so they will be copied.
        </Banner>
      )}
      {s.type_mismatches > 0 && (
        <Banner tone="warn" title={`${count(s.type_mismatches, "file")} whose content does not match the extension`}>
          They will be copied as-is and categorized by extension. Check them in the list (filter “Flagged”).
        </Banner>
      )}

      <Section title="Destinations">
        <div className="list">
          {plan.destinations.map((d, i) => (
            <div className="row" key={d.root}>
              <Icon name="drive" className="row-icon" />
              <div className="row-main">
                <div className="row-title">
                  {d.label} {!d.space_ok && <Badge tone="bad">Not enough space</Badge>}
                  {!d.index_complete && <Badge tone="warn">Partially readable</Badge>}
                </div>
                <div className="row-sub mono-path" title={d.root}>
                  [{i + 1}] {d.root}
                </div>
                <SpaceMeter total={d.available_bytes != null ? d.available_bytes + 0 : null} free={d.available_bytes} needed={d.required_bytes} />
              </div>
              <div className="row-meta">
                needs {bytes(d.required_bytes)}
                <br />
                <span className="muted">{bytes(d.available_bytes)} free</span>
              </div>
            </div>
          ))}
        </div>
      </Section>

      <Section title="Folder layout (first destination)">
        <TreePreview plan={plan} />
      </Section>

      <Section title="Files">
        <div className="tabs" role="tablist" aria-label="Filter files">
          {tabs.map((t) => (
            <button key={t.id} role="tab" aria-selected={filter === t.id} className={`tab ${filter === t.id ? "active" : ""}`} onClick={() => setFilter(t.id)}>
              {t.label} <span className="tab-n">{t.n.toLocaleString()}</span>
            </button>
          ))}
        </div>
        {rows.length === 0 ? (
          <Empty icon="check" title="Nothing in this category" />
        ) : (
          <div className="table" role="table" aria-label="Planned files">
            <div className="tr th" role="row">
              <span role="columnheader">Source file</span>
              <span role="columnheader">Size</span>
              <span role="columnheader">Type</span>
              <span role="columnheader">Plan</span>
            </div>
            {rows.slice(0, limit).map((it) => (
              <div className="tr" role="row" key={it.source.rel_path}>
                <span role="cell" className="mono-path" title={it.source.rel_path}>
                  {it.source.rel_path}
                </span>
                <span role="cell" className="num">
                  {bytes(it.source.size)}
                </span>
                <span role="cell">
                  {categoryLabel[it.category]}
                  {it.source.class.confidence === "mismatch" && (
                    <span className="tone-warn" title="Content does not match the extension">
                      {" "}
                      <Icon name="warn" size={12} />
                    </span>
                  )}
                </span>
                <span role="cell" className="plan-cell">
                  {it.excluded ? (
                    <span className="muted">
                      Excluded — {it.excluded.reason} <code>{it.excluded.pattern}</code>
                    </span>
                  ) : (
                    it.targets.map((t) => (
                      <span key={t.dest_index} className="plan-target">
                        {plan.destinations.length > 1 && <span className="muted">[{t.dest_index + 1}] </span>}
                        {t.action.kind === "copy" && (
                          <>
                            <Badge tone="info">Copy</Badge> <span className="mono-path">{t.target_rel}</span>
                          </>
                        )}
                        {t.action.kind === "skip_duplicate" && (
                          <>
                            <Badge tone="ok">Skip</Badge> {t.action.level === "verified_identical" ? "identical to" : "candidate, verified before skipping:"}{" "}
                            <span className="mono-path">{t.action.existing_rel}</span>
                          </>
                        )}
                        {t.action.kind === "conflict" && (
                          <>
                            <Badge tone="warn">Conflict</Badge> different file at <span className="mono-path">{t.action.existing_rel}</span>
                          </>
                        )}
                        {t.notes.map((n, k) => (
                          <span key={k} className="note">
                            {noteText(n)}
                          </span>
                        ))}
                      </span>
                    ))
                  )}
                </span>
              </div>
            ))}
            {rows.length > limit && (
              <div className="tr more">
                <Button variant="ghost" onClick={() => setLimit((l) => l + 1000)}>
                  Show {Math.min(1000, rows.length - limit).toLocaleString()} more of {rows.length.toLocaleString()}
                </Button>
              </div>
            )}
          </div>
        )}
      </Section>

      {plan.scan_issues.length > 0 && (
        <Section title="Not importable">
          <div className="list">
            {plan.scan_issues.map((i) => (
              <div className="row" key={i.rel_path}>
                <Icon name="warn" className="row-icon tone-warn" />
                <div className="row-main">
                  <div className="row-title mono-path">{i.rel_path}</div>
                  <div className="row-sub">{i.message}</div>
                </div>
              </div>
            ))}
          </div>
        </Section>
      )}
    </>
  );
}

export function NewImportPage({ go, initialSession }: { go: Navigate; initialSession?: string }) {
  const [step, setStep] = useState(initialSession ? 3 : 0);
  const [sessionId, setSessionId] = useState<string | null>(initialSession ?? null);
  const grants = useLoad(() => api.grants());
  const profiles = useLoad(() => api.profiles());
  const [source, setSource] = useState<string | null>(null);
  const [label, setLabel] = useState("");
  const [dests, setDests] = useState<string[]>([]);
  const [profileId, setProfileId] = useState("youtube");
  const [project, setProject] = useState("");
  const [proveDuplicates, setProveDuplicates] = useState(true);
  const [plan, setPlan] = useState<PlanView | null>(null);
  const [planning, setPlanning] = useState<PhaseProgress | null>(null);
  const [error, setError] = useState<UiError | null>(null);
  const [starting, setStarting] = useState(false);

  useEffect(() => {
    if (!planning) return;
    let un: (() => void) | undefined;
    void events.onPlanProgress((p) => setPlanning(p)).then((u) => (un = u));
    return () => un?.();
  }, [planning !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  const sources = (grants.data ?? []).filter((g) => g.kind === "source");
  const destinations = (grants.data ?? []).filter((g) => g.kind === "destination");
  const profile = profiles.data?.find((p) => p.id === profileId);

  const add = async (kind: "source" | "destination") => {
    setError(null);
    try {
      const g = await api.pickFolder(kind);
      if (!g) return;
      await grants.reload();
      if (kind === "source") {
        setSource(g.id);
        if (!label) setLabel(g.label);
      } else setDests((d) => (d.includes(g.id) ? d : [...d, g.id]));
    } catch (e) {
      setError(asUiError(e));
    }
  };

  const analyze = async () => {
    if (!source) return;
    setError(null);
    setPlan(null);
    setStep(2);
    setPlanning({ phase: "scanning_source", done: 0, total: 0 });
    try {
      const p = await api.createPlan({
        source_grant: source,
        destination_grants: dests,
        profile_id: profileId,
        project,
        label: label || null,
        quick: !proveDuplicates,
      });
      setPlan(p);
    } catch (e) {
      setError(asUiError(e));
    } finally {
      setPlanning(null);
    }
  };

  const start = async () => {
    if (!plan) return;
    setStarting(true);
    setError(null);
    try {
      const id = await api.startImport(plan.plan_id);
      setSessionId(id);
      setStep(3);
    } catch (e) {
      setError(asUiError(e));
    } finally {
      setStarting(false);
    }
  };

  const backToSetup = () => {
    if (plan) void api.discardPlan(plan.plan_id);
    setPlan(null);
    setStep(1);
  };

  const spaceProblem = plan?.destinations.some((d) => !d.space_ok) ?? false;
  const nothingToDo = plan ? plan.summary.to_copy === 0 && plan.summary.skip_duplicates === 0 : false;

  return (
    <div className="page">
      <PageHeader title="New import" subtitle="Choose a source, choose where it goes, review the plan, then copy and verify." />
      <Stepper step={step} />
      <ErrorBanner error={error ?? grants.error ?? profiles.error} onDismiss={() => setError(null)} />

      {step === 0 && (
        <Section title="1. Choose the source">
          <p className="hint">A memory card, camera, phone, drive or folder. MediaFlow only reads from it: nothing on the source is ever modified or deleted.</p>
          <FolderChoice
            grants={sources}
            selected={source ? [source] : []}
            multiple={false}
            onToggle={(id) => {
              setSource(id);
              const g = sources.find((x) => x.id === id);
              if (g) setLabel(g.label);
            }}
            onAdd={() => void add("source")}
            addLabel="Choose a card or folder…"
            kind="source"
          />
          <div className="field">
            <label htmlFor="label">Source name</label>
            <input id="label" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="e.g. A7S III card A" />
            <span className="field-hint">Used for the {"{camera}"} and {"{source}"} folder names.</span>
          </div>
          <div className="footer-actions">
            <Button variant="primary" disabled={!source} onClick={() => setStep(1)}>
              Continue
            </Button>
          </div>
        </Section>
      )}

      {step === 1 && (
        <>
          <Section title="2. Choose destinations">
            <p className="hint">Select one or more. Each destination receives its own verified copy.</p>
            <FolderChoice
              grants={destinations}
              selected={dests}
              multiple
              onToggle={(id) => setDests((d) => (d.includes(id) ? d.filter((x) => x !== id) : [...d, id]))}
              onAdd={() => void add("destination")}
              addLabel="Add a destination folder…"
              kind="destination"
            />
          </Section>
          <Section title="Profile">
            <div className="choice-list profiles" role="radiogroup" aria-label="Profile">
              {(profiles.data ?? []).map((p) => (
                <label key={p.id} className={`choice ${profileId === p.id ? "on" : ""}`}>
                  <input type="radio" name="profile" checked={profileId === p.id} onChange={() => setProfileId(p.id)} />
                  <span className="choice-main">
                    <span className="choice-title">
                      {p.name} {p.verify === "paranoid" && <Badge tone="neutral">Final re-read</Badge>}
                    </span>
                    <span className="choice-sub">{p.description}</span>
                    <code className="choice-code">
                      {p.template.folders}/{p.template.file_name}
                    </code>
                  </span>
                </label>
              ))}
            </div>
            {profile && (
              <details className="disclosure">
                <summary>What this profile excludes and how it handles conflicts</summary>
                <ul className="plain">
                  {profile.exclusions.map((x) => (
                    <li key={x.pattern}>
                      <code>{x.pattern}</code> — {x.reason}
                    </li>
                  ))}
                  <li>
                    Same name, different content:{" "}
                    {profile.conflicts === "flag" ? "not copied, flagged for you to decide" : 'copied under a new name such as "clip (2).mov"'}
                  </li>
                  <li>Identical file already at the destination: {profile.duplicates === "skip_verified" ? "skipped after SHA-256 verification" : "copied anyway"}</li>
                  <li>
                    Verification: {profile.verify === "paranoid" ? "each copy verified when written, then every copy re-read at the end" : "each copy re-read and verified before it gets its final name"}
                  </li>
                </ul>
              </details>
            )}
            <div className="field">
              <label htmlFor="project">Project name</label>
              <input id="project" value={project} onChange={(e) => setProject(e.target.value)} placeholder="e.g. Client X — Launch film" />
            </div>
            <label className="check">
              <input type="checkbox" checked={proveDuplicates} onChange={(e) => setProveDuplicates(e.target.checked)} />
              <span>
                Prove duplicates while analyzing <span className="muted">(reads candidate files; slower, but shows exact results before copying)</span>
              </span>
            </label>
          </Section>
          <div className="footer-actions">
            <Button icon="back" onClick={() => setStep(0)}>
              Back
            </Button>
            <Button variant="primary" disabled={dests.length === 0} onClick={() => void analyze()}>
              Analyze
            </Button>
          </div>
        </>
      )}

      {step === 2 && (
        <>
          {planning && (
            <Section title="Analyzing">
              <ProgressBar value={planning.total > 0 ? planning.done / planning.total : null} label="Analysis progress" />
              <p className="muted" aria-live="polite">
                {planning.phase === "scanning_source" && `Reading the source… ${planning.done.toLocaleString()} files found`}
                {planning.phase === "indexing_destination" && `Indexing destination… ${planning.done.toLocaleString()} files`}
                {planning.phase === "planning" && `Comparing ${planning.done.toLocaleString()} of ${planning.total.toLocaleString()} files`}
              </p>
            </Section>
          )}
          {plan && <PlanPreview plan={plan} />}
          {!planning && (
            <div className="footer-actions sticky">
              <Button icon="back" onClick={backToSetup}>
                Back
              </Button>
              {plan && (
                <>
                  {spaceProblem && <span className="tone-bad footer-note">Free up space or choose another destination first.</span>}
                  {nothingToDo && <span className="muted footer-note">Nothing to import.</span>}
                  <Button variant="primary" icon="import" disabled={spaceProblem || nothingToDo || starting} onClick={() => void start()}>
                    {plan.summary.to_copy > 0 ? `Copy and verify ${count(plan.summary.to_copy, "file")}` : "Verify duplicates and finish"}
                  </Button>
                </>
              )}
            </div>
          )}
        </>
      )}

      {step === 3 && sessionId && <RunView sessionId={sessionId} go={go} />}
      {step === 3 && !sessionId && <Empty icon="import" title="No import selected" />}
    </div>
  );
}
