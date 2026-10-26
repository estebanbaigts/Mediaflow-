import { useEffect, useState } from "react";
import { api, asUiError } from "../api";
import { Badge, Button, ErrorBanner, PageHeader, Section } from "../components/ui";
import { useLoad } from "../hooks";
import type { UiError } from "../types";

type Theme = "system" | "light" | "dark";

function readPref(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}
function writePref(key: string, v: string) {
  try {
    localStorage.setItem(key, v);
  } catch {
    /* preferences are a convenience only */
  }
}

export function applyPrefs() {
  const t = readPref("mf.theme", "system");
  const root = document.documentElement;
  if (t === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", t);
  root.toggleAttribute("data-reduce-motion", readPref("mf.motion", "on") === "off");
}

export function SettingsPage() {
  const info = useLoad(() => api.appInfo());
  const grants = useLoad(() => api.grants());
  const profiles = useLoad(() => api.profiles());
  const [theme, setTheme] = useState<Theme>(() => readPref("mf.theme", "system") as Theme);
  const [motion, setMotion] = useState(() => readPref("mf.motion", "on") === "on");
  const [error, setError] = useState<UiError | null>(null);

  useEffect(() => {
    writePref("mf.theme", theme);
    writePref("mf.motion", motion ? "on" : "off");
    applyPrefs();
  }, [theme, motion]);

  const revoke = async (id: string) => {
    try {
      await api.revokeGrant(id);
      await grants.reload();
    } catch (e) {
      setError(asUiError(e));
    }
  };

  return (
    <div className="page">
      <PageHeader title="Settings" />
      <ErrorBanner error={error ?? info.error ?? grants.error} onDismiss={() => setError(null)} />

      <Section title="Privacy">
        <ul className="plain">
          <li>
            <strong>Offline.</strong> {info.data?.network ?? "MediaFlow makes no network connections."} No account, no telemetry, no analytics, no cloud.
          </li>
          <li>
            <strong>Local data.</strong> History, fingerprints and reports are stored in one database on this computer
            {info.data?.library_path && (
              <>
                : <span className="mono-path">{info.data.library_path}</span>
              </>
            )}
            . It contains file names, sizes, dates and SHA-256 fingerprints — never file contents. It is not encrypted; protect it like your other documents.
          </li>
          <li>
            <strong>Your files.</strong> Sources are only read. Nothing is ever deleted or overwritten; MediaFlow removes only its own incomplete temporary files
            (named <code>.mfpart-…</code>).
          </li>
        </ul>
      </Section>

      <Section title="Authorized folders">
        <p className="hint">MediaFlow can only read or write inside folders you picked. Removing access does not touch any file.</p>
        <div className="list">
          {(grants.data ?? []).map((g) => (
            <div className="row" key={g.id}>
              <div className="row-main">
                <div className="row-title">
                  {g.label} <Badge tone="neutral">{g.kind === "source" ? "Read" : "Read & write"}</Badge> {!g.available && <Badge tone="warn">Not connected</Badge>}
                </div>
                <div className="row-sub mono-path">{g.path}</div>
              </div>
              <Button variant="ghost" onClick={() => void revoke(g.id)}>
                Remove access
              </Button>
            </div>
          ))}
          {grants.data?.length === 0 && <p className="muted">None yet.</p>}
        </div>
      </Section>

      <Section title="Profiles">
        <p className="hint">Built-in profiles. Editing and creating custom profiles is not available in this version yet.</p>
        <div className="list">
          {(profiles.data ?? []).map((p) => (
            <details className="row disclosure-row" key={p.id}>
              <summary>
                <span className="row-title">{p.name}</span> <code>{p.template.folders}/{p.template.file_name}</code>
              </summary>
              <ul className="plain">
                <li>{p.description}</li>
                <li>Verification: {p.verify === "paranoid" ? "verify on write + final re-read of every copy" : "verify on write"}</li>
                <li>Same name, different content: {p.conflicts === "flag" ? "flag, do not copy" : "keep both (rename the new copy)"}</li>
                <li>Exclusions: {p.exclusions.map((x) => x.pattern).join(", ")}</li>
                {p.category_overrides.map((o) => (
                  <li key={o.pattern}>
                    <code>{o.pattern}</code> filed as {o.category}
                  </li>
                ))}
              </ul>
            </details>
          ))}
        </div>
      </Section>

      <Section title="Appearance">
        <div className="field inline">
          <label htmlFor="theme">Theme</label>
          <select id="theme" value={theme} onChange={(e) => setTheme(e.target.value as Theme)}>
            <option value="system">Match system</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </div>
        <label className="check">
          <input type="checkbox" checked={motion} onChange={(e) => setMotion(e.target.checked)} />
          <span>Animations</span>
        </label>
      </Section>

      <Section title="About">
        <p className="muted">
          MediaFlow {info.data?.version} · Apache-2.0 · <Button variant="ghost" onClick={() => void info.reload()}>Refresh</Button>
        </p>
      </Section>
    </div>
  );
}
