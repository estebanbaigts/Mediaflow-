import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Icon, type IconName } from "./components/Icon";
import { RunsProvider, useRuns } from "./store";
import { OverviewPage } from "./pages/Overview";
import { NewImportPage } from "./pages/NewImport";
import { ProjectsPage } from "./pages/Projects";
import { LibraryPage } from "./pages/Library";
import { DuplicatesPage } from "./pages/Duplicates";
import { VerifyPage } from "./pages/Verify";
import { HistoryPage } from "./pages/History";
import { SettingsPage } from "./pages/Settings";

export type Route =
  | { page: "overview" }
  | { page: "import"; sessionId?: string }
  | { page: "projects" }
  | { page: "library" }
  | { page: "duplicates" }
  | { page: "verify" }
  | { page: "history"; sessionId?: string; project?: string }
  | { page: "settings" };

export type Navigate = (r: Route) => void;

const NAV: { page: Route["page"]; label: string; icon: IconName; shortcut: string }[] = [
  { page: "overview", label: "Overview", icon: "overview", shortcut: "1" },
  { page: "import", label: "New import", icon: "import", shortcut: "2" },
  { page: "projects", label: "Projects", icon: "projects", shortcut: "3" },
  { page: "library", label: "Media library", icon: "library", shortcut: "4" },
  { page: "duplicates", label: "Duplicates", icon: "duplicates", shortcut: "5" },
  { page: "verify", label: "Verification", icon: "verify", shortcut: "6" },
  { page: "history", label: "History", icon: "history", shortcut: "7" },
  { page: "settings", label: "Settings", icon: "settings", shortcut: "," },
];

function Sidebar({ route, go }: { route: Route; go: Navigate }) {
  const { runs } = useRuns();
  const active = Object.values(runs).filter((r) => r.progress && !r.finished).length;
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const onKey = (e: KeyboardEvent, i: number) => {
    const n = NAV.length;
    const to = e.key === "ArrowDown" ? (i + 1) % n : e.key === "ArrowUp" ? (i - 1 + n) % n : e.key === "Home" ? 0 : e.key === "End" ? n - 1 : -1;
    if (to >= 0) {
      e.preventDefault();
      refs.current[to]?.focus();
    }
  };
  return (
    <nav className="sidebar" aria-label="Main">
      <div className="brand">
        <span className="brand-mark" aria-hidden="true" />
        <span>MediaFlow</span>
      </div>
      <ul role="list">
        {NAV.map((n, i) => (
          <li key={n.page}>
            <button
              ref={(el) => {
                refs.current[i] = el;
              }}
              className={`nav-item ${route.page === n.page ? "active" : ""}`}
              aria-current={route.page === n.page ? "page" : undefined}
              onClick={() => go({ page: n.page } as Route)}
              onKeyDown={(e) => onKey(e, i)}
              title={`${n.label} (Ctrl/⌘ ${n.shortcut})`}
            >
              <Icon name={n.icon} />
              <span className="nav-label">{n.label}</span>
              {n.page === "import" && active > 0 && (
                <span className="nav-count" aria-label={`${active} running`}>
                  {active}
                </span>
              )}
            </button>
          </li>
        ))}
      </ul>
      <div className="sidebar-foot">
        <Icon name="lock" />
        <span>Offline · local only</span>
      </div>
    </nav>
  );
}

function Shell() {
  const [route, setRoute] = useState<Route>({ page: "overview" });
  const mainRef = useRef<HTMLElement>(null);
  const go = useCallback<Navigate>((r) => {
    setRoute(r);
    requestAnimationFrame(() => mainRef.current?.focus());
  }, []);

  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
      const n = NAV.find((x) => x.shortcut === e.key);
      if (n) {
        e.preventDefault();
        go({ page: n.page } as Route);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go]);

  return (
    <div className="app">
      <Sidebar route={route} go={go} />
      <main className="content" ref={mainRef} tabIndex={-1} aria-live="polite">
        {route.page === "overview" && <OverviewPage go={go} />}
        {route.page === "import" && <NewImportPage go={go} initialSession={route.sessionId} key={route.sessionId ?? "new"} />}
        {route.page === "projects" && <ProjectsPage go={go} />}
        {route.page === "library" && <LibraryPage />}
        {route.page === "duplicates" && <DuplicatesPage go={go} />}
        {route.page === "verify" && <VerifyPage />}
        {route.page === "history" && <HistoryPage go={go} sessionId={route.sessionId} project={route.project} key={route.sessionId ?? route.project ?? "all"} />}
        {route.page === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}

export function App() {
  return (
    <RunsProvider>
      <Shell />
    </RunsProvider>
  );
}
