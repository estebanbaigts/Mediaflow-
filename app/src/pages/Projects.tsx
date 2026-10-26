import { api } from "../api";
import type { Navigate } from "../App";
import { Icon } from "../components/Icon";
import { Badge, Empty, ErrorBanner, PageHeader } from "../components/ui";
import { bytes, count, relative } from "../format";
import { useLoad } from "../hooks";
import type { SessionInfo } from "../types";

export function ProjectsPage({ go }: { go: Navigate }) {
  const { data, error, loading } = useLoad(() => api.sessions(5000));
  const groups = new Map<string, SessionInfo[]>();
  for (const s of data ?? []) {
    const k = s.project || "Untitled";
    groups.set(k, [...(groups.get(k) ?? []), s]);
  }
  const list = [...groups.entries()].sort((a, b) => (b[1][0]?.created_at ?? "").localeCompare(a[1][0]?.created_at ?? ""));
  return (
    <div className="page">
      <PageHeader title="Projects" subtitle="Imports grouped by project name." />
      <ErrorBanner error={error} />
      {loading && !data ? (
        <div className="skeleton-list" aria-busy="true" aria-label="Loading" />
      ) : list.length === 0 ? (
        <Empty icon="projects" title="No projects yet">
          Give an import a project name and it appears here.
        </Empty>
      ) : (
        <div className="list">
          {list.map(([name, ss]) => {
            const files = ss.reduce((n, s) => n + s.counts.done, 0);
            const size = ss.reduce((n, s) => n + s.counts.bytes_done, 0);
            const problems = ss.filter((s) => s.state !== "completed" && s.state !== "cancelled").length;
            return (
              <button key={name} className="row row-button" onClick={() => go({ page: "history", project: name === "Untitled" ? "" : name })}>
                <Icon name="projects" className="row-icon" />
                <div className="row-main">
                  <div className="row-title">{name}</div>
                  <div className="row-sub">
                    {count(ss.length, "import")} · {count(files, "file")} · {bytes(size)} · last {relative(ss[0]?.created_at)}
                  </div>
                </div>
                {problems > 0 && <Badge tone="warn">{count(problems, "import")} need attention</Badge>}
                <Icon name="chevron" className="row-chevron" />
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
