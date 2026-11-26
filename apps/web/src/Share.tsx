import { useEffect, useState } from "react";
import { api, ApiError, Cell, Column, SharedView } from "./api";
import { day, dayStart, FieldChooser } from "./Access";

// Share links: a read-only view of one record, or of every record its maker may see, for
// people without an account. The server decides what the link shows (doc: mt-perm share.rs
// and mt-server access.rs); this file makes links and shows them.

export function shareUrl(token: string): string {
  return `${window.location.origin}${window.location.pathname}#/share/${token}`;
}

/// In the record panel: make a link to this record or to the table.
export function SharePanel({ tableId, recordId, fields, onClose }: { tableId: string; recordId: string; fields: Column[]; onClose: () => void }) {
  const [what, setWhat] = useState<"record" | "table">("record");
  const [mode, setMode] = useState("all");
  const [left, setLeft] = useState<string[]>([]);
  const [expires, setExpires] = useState("");
  const [url, setUrl] = useState("");
  const [message, setMessage] = useState("");
  const shareable = fields
    .filter((f) => f.dataType !== "link")
    .map((f) => ({ id: f.id, name: f.name, dataType: f.dataType, type: f.type, generated: f.generated, options: null }));
  const create = async () => {
    setMessage("");
    try {
      const made = await api.createShare(tableId, {
        record: what === "record" ? recordId : undefined,
        fields: mode === "except" && left.length ? { except: left } : undefined,
        expiresAt: dayStart(expires),
      });
      setUrl(shareUrl(made.token));
    } catch (e) {
      setMessage((e as ApiError).message);
    }
  };
  return (
    <div className="share-panel" data-testid="share-panel">
      <div className="form-row">
        <select value={what} onChange={(e) => setWhat(e.target.value as "record" | "table")} data-testid="share-what">
          <option value="record">this record</option>
          <option value="table">every record you can see in this table</option>
        </select>
        <span>showing</span>
        <FieldChooser fields={shareable} mode={mode} chosen={left} onMode={setMode} onChosen={setLeft} testId="share-fields" modes={["all", "except"]} />
      </div>
      <div className="form-row">
        <label className="inline small">expires <input type="date" value={expires} onChange={(e) => setExpires(e.target.value)} data-testid="share-expires" /></label>
        <button onClick={create} data-testid="create-share">Create link</button>
        <button className="ghost small" onClick={onClose}>Close</button>
      </div>
      <p className="muted small">Read only. Links to other tables, confidential and protected fields are never shown; masked values stay masked.</p>
      {url && (
        <div className="notice ok">
          Anyone with this link can see it{expires ? ` until ${expires}` : ""}:
          <input className="share-url" readOnly value={url} onFocus={(e) => e.target.select()} data-testid="share-url" />
        </div>
      )}
      {message && <div className="notice refused" data-testid="share-message">{message}</div>}
    </div>
  );
}

function Value({ cell }: { cell?: Cell }) {
  if (!cell || cell.hidden) return <span className="muted">—</span>;
  if (cell.masked) return <span className="masked" title="Masked">🔒 {cell.d}</span>;
  if (cell.error) return <span className="cell-error">{cell.d}</span>;
  return <span>{cell.d}</span>;
}

/// The page a link opens: no account, no navigation, read only.
export function SharedPage({ token }: { token: string }) {
  const [view, setView] = useState<SharedView | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    api
      .openShare(token)
      .then(setView)
      .catch((e: ApiError) => setError(e.status === 404 ? "This link does not exist." : e.message.charAt(0).toUpperCase() + e.message.slice(1) + "."));
  }, [token]);
  return (
    <div className="shared">
      <header className="topbar">
        <div className="brand">
          <span className="logo">▦</span> agenttable <span className="muted small">· shared with you</span>
        </div>
      </header>
      <main className="shared-main">
        {error && <div className="notice refused" data-testid="shared-error">{error}</div>}
        {view && (
          <>
            <div className="muted small">{view.table}</div>
            <h1 data-testid="shared-title">{view.record ?? view.table}</h1>
            <p className="muted small" data-testid="shared-by">
              Shared by {view.sharedBy}
              {view.expiresAt ? ` · until ${day(view.expiresAt)}` : ""} · read only
            </p>
            {view.record ? (
              <div className="card shared-record" data-testid="shared-record">
                {view.columns.map((c) => (
                  <div className="field-row" key={c.id}>
                    <div className="field-label">{c.name}<span className="type-tag">{c.typeLabel}</span></div>
                    <div className="field-value" data-testid={`shared-${c.name}`}><Value cell={view.records[0]?.cells[c.id]} /></div>
                  </div>
                ))}
              </div>
            ) : (
              <div className="grid-wrap">
                <table className="grid" data-testid="shared-grid">
                  <thead>
                    <tr>{view.columns.map((c) => <th key={c.id} className={c.numeric ? "num" : ""}>{c.name}</th>)}</tr>
                  </thead>
                  <tbody>
                    {view.records.map((r) => (
                      <tr key={r.id} data-testid="shared-row">
                        {view.columns.map((c) => <td key={c.id} className={c.numeric ? "num" : ""}><Value cell={r.cells[c.id]} /></td>)}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </>
        )}
      </main>
    </div>
  );
}
