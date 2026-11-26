import { useCallback, useEffect, useMemo, useState } from "react";
import { api, ApiError, Cell, Column, QueryResult, Row } from "./api";
import RecordPanel from "./RecordPanel";
import FieldPicker from "./FieldPicker";
import { FieldEditor } from "./Build";

type Filter = { field: string; op: string; text: string; me: boolean };

const OPERATORS: Record<string, string[]> = {
  text: ["contains", "eq", "ne", "starts_with", "is_empty", "is_not_empty"],
  long_text: ["contains", "is_empty", "is_not_empty"],
  number: ["eq", "ne", "gt", "ge", "lt", "le", "is_empty", "is_not_empty"],
  money: ["gt", "ge", "lt", "le", "eq", "is_empty", "is_not_empty"],
  percent: ["gt", "ge", "lt", "le", "eq"],
  date: ["eq", "lt", "le", "gt", "ge", "is_empty", "is_not_empty"],
  datetime: ["lt", "le", "gt", "ge", "is_empty", "is_not_empty"],
  bool: ["eq"],
  reference: ["eq", "ne", "is_empty", "is_not_empty"],
  principal: ["eq", "is_empty", "is_not_empty"],
  link: ["is_empty", "is_not_empty"],
};

const OP_LABEL: Record<string, string> = {
  contains: "contains",
  eq: "is",
  ne: "is not",
  starts_with: "starts with",
  is_empty: "is empty",
  is_not_empty: "is not empty",
  gt: ">",
  ge: "≥",
  lt: "<",
  le: "≤",
};

export function CellView({ column, cell }: { column: Column; cell?: Cell }) {
  if (!cell) return <span className="muted">—</span>;
  if (cell.error) return <span className="cell-error" title={cell.error}>{cell.d}</span>;
  if (cell.masked) return <span className="masked" title="Masked: you may not see this value in clear">🔒 {cell.d}</span>;
  if (!cell.d) return null;
  if (column.dataType === "bool") return <span className={cell.d === "Yes" ? "yes" : "no"}>{cell.d === "Yes" ? "✓" : "—"}</span>;
  if (column.type === "lifecycle_state" || (column.dataType === "reference" && column.type === "single_select")) {
    return <span className={`pill pill-${cell.d.toLowerCase().replace(/[^a-z]/g, "")}`}>{cell.d}</span>;
  }
  if (column.dataType === "link") {
    return (
      <span className="chips">
        {(cell.items ?? [cell.d]).map((t, i) => (
          <span className="chip" key={i}>{t}</span>
        ))}
      </span>
    );
  }
  if (column.type === "rating") {
    const n = Number(cell.d) || 0;
    return <span className="stars" title={`${n} of 5`}>{"★".repeat(n)}<span className="muted">{"★".repeat(Math.max(0, 5 - n))}</span></span>;
  }
  return <span>{cell.d}</span>;
}

export function ColumnHeader({ column }: { column: Column }) {
  return (
    <span className="col-head" title={column.formula ? `${column.typeLabel}: ${column.formula}` : column.typeLabel}>
      {column.computed && <span className="fx">ƒ</span>}
      {column.name}
      {column.masked && <span className="lock" title="Masked">🔒</span>}
    </span>
  );
}

export default function TablePage({ tableId, onChanged }: { tableId: string; onChanged: () => void }) {
  const [result, setResult] = useState<QueryResult | null>(null);
  const [error, setError] = useState<ApiError | null>(null);
  const [search, setSearch] = useState("");
  const [filters, setFilters] = useState<Filter[]>([]);
  const [sort, setSort] = useState<{ field: string; desc: boolean } | null>(null);
  const [group, setGroup] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [picking, setPicking] = useState(false);
  const [editing, setEditing] = useState<Column | null>(null);

  const load = useCallback(() => {
    const spec = {
      search,
      filters: filters
        .filter((f) => f.field && f.op)
        .map((f) => {
          const needsValue = !["is_empty", "is_not_empty"].includes(f.op);
          return { field: f.field, op: f.op, me: f.me, value: needsValue && !f.me ? (f.text.startsWith("rfi_") ? { value: { type: "reference", value: f.text } } : f.text === "true" || f.text === "false" ? { value: { type: "bool", value: f.text === "true" } } : { text: f.text }) : undefined };
        }),
      sort: sort ? [sort] : [],
      group: group || null,
    };
    api
      .query(tableId, spec)
      .then((r) => {
        setResult(r);
        setError(null);
      })
      .catch((e: ApiError) => setError(e));
  }, [tableId, search, filters, sort, group]);

  useEffect(() => {
    const timer = window.setTimeout(load, search ? 200 : 0);
    return () => window.clearTimeout(timer);
  }, [load, search]);

  const columns = result?.table.columns ?? [];
  const byId = useMemo(() => new Map((result?.rows ?? []).map((r) => [r.id, r])), [result]);

  if (error && !result) {
    return (
      <div className="page">
        <div className="notice refused" data-testid="no-access">
          <h2>No access</h2>
          <p>{error.message}</p>
          <ExplainList details={error.details} />
        </div>
      </div>
    );
  }
  if (!result) return <div className="page muted">Loading…</div>;

  const table = result.table;
  const totalsRow = (totals: Record<string, string>, label: string, key: string) => (
    <tr className="totals" key={key}>
      {columns.map((c, i) => (
        <td key={c.id} className={c.numeric ? "num" : ""}>
          {i === 0 ? <span className="muted">{label}</span> : totals[c.id] ?? ""}
        </td>
      ))}
    </tr>
  );
  const rowView = (row: Row) => (
    <tr key={row.id} onClick={() => setOpen(row.id)} className={open === row.id ? "selected" : ""} data-testid="grid-row">
      {columns.map((c) => (
        <td key={c.id} className={c.numeric ? "num" : ""}>
          <CellView column={c} cell={row.cells[c.id]} />
        </td>
      ))}
    </tr>
  );

  const groupColumn = columns.find((c) => c.id === group);

  return (
    <div className="page table-page">
      <div className="page-head">
        <div>
          <h1 data-testid="table-title">{table.name}</h1>
          <p className="muted">{table.description ? `${table.description} · ` : ""}<span data-testid="row-count">{result.count}</span> record(s) you can see</p>
        </div>
        <div className="actions">
          {table.canManageSchema && (
            <a className="button secondary" href={`#/table/${tableId}/process`} data-testid="design-process">
              Process
            </a>
          )}
          {table.canManageSchema && (
            <button className="secondary" onClick={() => setPicking(true)} data-testid="add-field">
              + Field
            </button>
          )}
          {table.canCreate && (
            <button onClick={() => setCreating(true)} data-testid="new-record">
              + New record
            </button>
          )}
        </div>
      </div>

      <div className="toolbar">
        <input className="search" placeholder="Search…" value={search} onChange={(e) => setSearch(e.target.value)} data-testid="search" />
        <button className="secondary" onClick={() => setFilters([...filters, { field: columns[0]?.id ?? "", op: "", text: "", me: false }])} data-testid="add-filter">
          + Filter
        </button>
        <label className="inline">
          Sort
          <select value={sort ? `${sort.field}:${sort.desc}` : ""} onChange={(e) => { const [f, d] = e.target.value.split(":"); setSort(f ? { field: f, desc: d === "true" } : null); }} data-testid="sort">
            <option value="">—</option>
            {columns.filter((c) => c.visibility === "visible").flatMap((c) => [
              <option key={`${c.id}a`} value={`${c.id}:false`}>{c.name} ↑</option>,
              <option key={`${c.id}d`} value={`${c.id}:true`}>{c.name} ↓</option>,
            ])}
          </select>
        </label>
        <label className="inline">
          Group by
          <select value={group} onChange={(e) => setGroup(e.target.value)} data-testid="group-by">
            <option value="">—</option>
            {columns.filter((c) => c.visibility === "visible" && !["link", "long_text"].includes(c.dataType)).map((c) => (
              <option key={c.id} value={c.id}>{c.name}</option>
            ))}
          </select>
        </label>
      </div>

      {filters.length > 0 && (
        <div className="filters">
          {filters.map((f, index) => {
            const column = columns.find((c) => c.id === f.field);
            const ops = OPERATORS[column?.dataType ?? "text"] ?? ["eq"];
            const update = (patch: Partial<Filter>) => setFilters(filters.map((x, i) => (i === index ? { ...x, ...patch } : x)));
            return (
              <div className="filter" key={index} data-testid="filter-row">
                <select value={f.field} onChange={(e) => update({ field: e.target.value, op: "", text: "" })} data-testid="filter-field">
                  {columns.filter((c) => c.visibility === "visible").map((c) => (
                    <option key={c.id} value={c.id}>{c.name}</option>
                  ))}
                </select>
                <select value={f.op} onChange={(e) => update({ op: e.target.value })} data-testid="filter-op">
                  <option value="">…</option>
                  {ops.map((op) => (
                    <option key={op} value={op}>{OP_LABEL[op] ?? op}</option>
                  ))}
                </select>
                {!["is_empty", "is_not_empty"].includes(f.op) && column?.dataType === "principal" && (
                  <label className="inline"><input type="checkbox" checked={f.me} onChange={(e) => update({ me: e.target.checked })} /> me</label>
                )}
                {!["is_empty", "is_not_empty"].includes(f.op) && !f.me && (
                  <FilterValue tableId={tableId} column={column} value={f.text} onChange={(text) => update({ text })} />
                )}
                <button className="ghost" onClick={() => setFilters(filters.filter((_, i) => i !== index))} title="Remove">✕</button>
              </div>
            );
          })}
        </div>
      )}

      {error && <div className="notice refused">{error.message}</div>}

      {table.hidden.length > 0 && (
        <div className="notice hidden-fields" data-testid="hidden-fields">
          🔒 {table.hidden.length} field(s) hidden for you:{" "}
          {table.hidden.map((h) => (
            <span key={h.id} className="hidden-field">
              <b>{h.name}</b> ({h.reason})
            </span>
          ))}
        </div>
      )}

      <div className="grid-wrap">
        <table className="grid" data-testid="grid">
          <thead>
            <tr>
              {columns.map((c) => (
                <th
                  key={c.id}
                  className={`${c.numeric ? "num" : ""} ${table.canManageSchema ? "editable-head" : ""}`}
                  onClick={table.canManageSchema ? () => setEditing(c) : undefined}
                  data-testid={`head-${c.name}`}
                >
                  <ColumnHeader column={c} />
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {result.groups.length > 0
              ? result.groups.flatMap((g) => [
                  <tr key={`g-${g.key}`} className="group-head" data-testid="group-head">
                    <td colSpan={columns.length}>
                      <b>{groupColumn?.name}: {g.key}</b> <span className="muted">· {g.count} record(s)</span>
                    </td>
                  </tr>,
                  ...g.rows.map((id) => byId.get(id)).filter((r): r is Row => !!r).map(rowView),
                  totalsRow(g.totals, "Subtotal", `t-${g.key}`),
                ])
              : result.rows.map(rowView)}
            {result.rows.length === 0 && (
              <tr>
                <td colSpan={Math.max(1, columns.length)} className="muted empty">No records.</td>
              </tr>
            )}
          </tbody>
          {result.rows.length > 0 && <tfoot>{totalsRow(result.totals, "Total", "total")}</tfoot>}
        </table>
      </div>

      {(open || creating) && (
        <RecordPanel
          tableId={tableId}
          table={table}
          recordId={creating ? null : open}
          onClose={() => { setOpen(null); setCreating(false); }}
          onSaved={(id) => { setCreating(false); setOpen(id); load(); onChanged(); }}
          onDeleted={() => { setOpen(null); load(); onChanged(); }}
        />
      )}
      {picking && <FieldPicker tableId={tableId} onClose={() => setPicking(false)} onCreated={() => { setPicking(false); load(); onChanged(); }} />}
      {editing && <FieldEditor tableId={tableId} column={editing} columns={columns} onClose={() => setEditing(null)} onSaved={() => { setEditing(null); load(); onChanged(); }} />}
    </div>
  );
}

function FilterValue({ tableId, column, value, onChange }: { tableId: string; column?: Column; value: string; onChange: (v: string) => void }) {
  const [options, setOptions] = useState<{ value: string; label: string }[] | null>(null);
  useEffect(() => {
    if (column && (column.dataType === "reference" || column.dataType === "bool")) {
      if (column.dataType === "bool") {
        setOptions([{ value: "true", label: "Yes" }, { value: "false", label: "No" }]);
        return;
      }
      api.options(tableId, column.id).then((list) =>
        setOptions(list.map((o) => ({ value: typeof o.value === "string" ? o.value : String((o.value as { value: unknown }).value), label: o.path ?? o.label }))),
      );
    } else {
      setOptions(null);
    }
  }, [tableId, column]);
  if (options) {
    return (
      <select value={value} onChange={(e) => onChange(e.target.value)} data-testid="filter-value">
        <option value="">…</option>
        {options.map((o) => (
          <option key={o.value} value={o.value}>{o.label}</option>
        ))}
      </select>
    );
  }
  return <input value={value} onChange={(e) => onChange(e.target.value)} placeholder="value" data-testid="filter-value" />;
}

export function ExplainList({ details }: { details: unknown }) {
  const d = details as { chain?: unknown[]; refusals?: { principal: string; reason: string; gives: string | null; on: string | null; code: string }[] } | undefined;
  if (!d || !Array.isArray(d.refusals) || d.refusals.length === 0) return null;
  // Grants for other tables say little when a grant for this table came close.
  const specific = d.refusals.filter((r) => r.code !== "elsewhere");
  const shown = specific.length > 0 ? specific : d.refusals;
  const skipped = d.refusals.length - shown.length;
  return (
    <ul className="reasons">
      {skipped > 0 && <li className="muted small" style={{ listStyle: "none", marginLeft: "-1.2rem" }}>({skipped} grant(s) for other places not shown)</li>}
      {shown.map((r, i) => (
        <li key={i}>
          <b>{r.principal}</b>: {r.reason}
          {r.gives && <span className="muted"> (grant of {r.gives}{r.on ? ` on ${r.on}` : ""})</span>}
        </li>
      ))}
    </ul>
  );
}
