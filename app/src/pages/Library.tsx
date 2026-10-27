import { useEffect, useState } from "react";
import { api, asUiError } from "../api";
import { Icon } from "../components/Icon";
import { Empty, ErrorBanner, Mono, PageHeader } from "../components/ui";
import { bytes, categoryLabel, dateTime } from "../format";
import type { Category, LibraryRow, UiError } from "../types";

export function LibraryPage() {
  const [q, setQ] = useState("");
  const [rows, setRows] = useState<LibraryRow[] | null>(null);
  const [error, setError] = useState<UiError | null>(null);
  useEffect(() => {
    const t = setTimeout(() => {
      api
        .library(q)
        .then((r) => (setRows(r), setError(null)))
        .catch((e) => setError(asUiError(e)));
    }, 180);
    return () => clearTimeout(t);
  }, [q]);
  return (
    <div className="page">
      <PageHeader title="Media library" subtitle="Every copy MediaFlow has written or verified, with its SHA-256 fingerprint." />
      <ErrorBanner error={error} />
      <div className="search">
        <Icon name="search" />
        <input aria-label="Search the library" placeholder="Search by file name, folder or project" value={q} onChange={(e) => setQ(e.target.value)} />
      </div>
      {rows == null ? (
        <div className="skeleton-list" aria-busy="true" aria-label="Loading" />
      ) : rows.length === 0 ? (
        <Empty icon="library" title={q ? "No matching files" : "The library is empty"}>
          {q ? "Try another search." : "Files appear here once an import has copied and verified them."}
        </Empty>
      ) : (
        <div className="table" role="table" aria-label="Library">
          <div className="tr th" role="row">
            <span role="columnheader">File</span>
            <span role="columnheader">Size</span>
            <span role="columnheader">Type</span>
            <span role="columnheader">Project · verified</span>
          </div>
          {rows.map((r) => (
            <div className="tr" role="row" key={`${r.dest_root}/${r.rel_path}`}>
              <span role="cell" className="plan-cell">
                <span className="mono-path" title={`${r.dest_root}/${r.rel_path}`}>
                  {r.rel_path}
                </span>
                <span className="note">
                  {r.dest_root} · from {r.source_label}/{r.source_rel}
                </span>
              </span>
              <span role="cell" className="num">
                {bytes(r.size)}
              </span>
              <span role="cell">{categoryLabel[r.category as Category] ?? r.category}</span>
              <span role="cell" className="plan-cell">
                <span>{r.project || "Untitled"}</span>
                <span className="note">
                  {dateTime(r.verified_at)} · <Mono title={r.sha256}>{r.sha256.slice(0, 12)}…</Mono>
                </span>
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
