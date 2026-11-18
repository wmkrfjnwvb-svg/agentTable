import { useEffect, useState } from "react";
import { api, ApiError, CellInput, Column, Explanation, Option, RecordDetail, TableInfo, Value } from "./api";
import { CellView, ExplainList } from "./TablePage";

type Draft = Record<string, CellInput>;

/// The text an input starts with: the stored value where it reads back exactly.
function initialText(column: Column): string {
  const cell = column.cell;
  if (!cell || !cell.v) return "";
  const v = cell.v;
  if (v.type === "money") {
    const m = v.value as { amount: string; currency: string };
    return `${m.amount} ${m.currency}`;
  }
  if (v.type === "number" || v.type === "date" || v.type === "text" || v.type === "long_text") return String(v.value ?? "");
  return cell.d;
}

function selected(column: Column): string[] {
  const v = column.cell?.v;
  if (!v) return [];
  if (v.type === "reference") return [String(v.value)];
  if (Array.isArray(v.value)) return (v.value as unknown[]).map(String);
  return [];
}

function optionValue(o: Option): string {
  if (typeof o.value === "string") return o.value;
  const inner = (o.value as Value).value;
  return Array.isArray(inner) ? String(inner[0]) : String(inner);
}

function Editor({ tableId, column, draft, setDraft, error }: { tableId: string; column: Column; draft: Draft; setDraft: (d: Draft) => void; error?: string }) {
  const [options, setOptions] = useState<Option[]>([]);
  const kind = column.dataType;
  const choice = ["reference", "multi_reference", "link", "principal"].includes(kind);
  useEffect(() => {
    if (choice) api.options(tableId, column.id).then(setOptions).catch(() => setOptions([]));
  }, [tableId, column.id, choice]);

  const set = (input: CellInput) => setDraft({ ...draft, [column.id]: input });
  const current = draft[column.id];
  const testId = `edit-${column.name}`;

  if (kind === "bool") {
    const checked = current && "value" in current ? Boolean(current.value?.value) : column.cell?.v?.value === true;
    return <input type="checkbox" checked={checked} onChange={(e) => set({ value: { type: "bool", value: e.target.checked } })} data-testid={testId} />;
  }
  if (choice) {
    const chosen = current && "value" in current ? (current.value ? (Array.isArray(current.value.value) ? (current.value.value as unknown[]).map(String) : [String(current.value.value)]) : []) : selected(column);
    const single = kind === "reference" || kind === "principal" || column.maxItems === 1;
    const wrap = (ids: string[]): CellInput => {
      if (ids.length === 0) return { value: null };
      if (kind === "reference") return { value: { type: "reference", value: ids[0] } };
      if (kind === "principal") return { value: { type: "principal", value: ids } };
      if (kind === "multi_reference") return { value: { type: "multi_reference", value: ids } };
      return { value: { type: "link", value: ids } };
    };
    if (single) {
      return (
        <select value={chosen[0] ?? ""} onChange={(e) => set(wrap(e.target.value ? [e.target.value] : []))} data-testid={testId} className={error ? "invalid" : ""}>
          <option value="">—</option>
          {options.map((o) => (
            <option key={optionValue(o)} value={optionValue(o)}>
              {o.depth && o.depth > 1 ? "  ".repeat(o.depth - 1) : ""}
              {o.label}
            </option>
          ))}
        </select>
      );
    }
    return (
      <div className="multi" data-testid={testId}>
        {options.map((o) => {
          const id = optionValue(o);
          const on = chosen.includes(id);
          return (
            <label key={id} className={`chip-toggle ${on ? "on" : ""}`}>
              <input type="checkbox" checked={on} onChange={() => set(wrap(on ? chosen.filter((x) => x !== id) : [...chosen, id]))} />
              {o.label}
            </label>
          );
        })}
      </div>
    );
  }
  const text = current && "text" in current ? current.text : initialText(column);
  const type = kind === "date" ? "date" : "text";
  if (kind === "long_text") {
    return <textarea value={text} onChange={(e) => set({ text: e.target.value })} data-testid={testId} className={error ? "invalid" : ""} rows={3} />;
  }
  return (
    <input
      type={type}
      value={text}
      onChange={(e) => set({ text: e.target.value })}
      placeholder={kind === "money" ? "e.g. 120.00 USD" : kind === "percent" ? "e.g. 8%" : ""}
      data-testid={testId}
      className={error ? "invalid" : ""}
    />
  );
}

function Decision({ label, explanation, note }: { label: string; explanation?: Explanation; note?: string }) {
  if (!explanation) return null;
  return (
    <div className={`decision ${explanation.allowed ? "allowed" : "refused"}`} data-testid={`decision-${label}`}>
      <div className="decision-head">
        {explanation.allowed ? "✓" : "✕"} <b>{label}</b>: {explanation.allowed ? "allowed" : "not allowed"}
      </div>
      {explanation.allowed
        ? explanation.chain.slice(0, 2).map((c, i) => (
            <div key={i} className="muted small">
              through {c.viaGroup ? `the group ${c.holder}` : c.holder}: <code>{c.capability}</code>
              {c.set ? ` (set “${c.set}”)` : ""} on {c.on}
              {c.scope && c.scope !== "all records" ? ` — ${c.scope}` : ""}
              {c.reason ? ` · “${c.reason}”` : ""}
            </div>
          ))
        : <ExplainList details={explanation} />}
      {note && <div className="small note">{note}</div>}
    </div>
  );
}

export default function RecordPanel({
  tableId,
  table,
  recordId,
  onClose,
  onSaved,
  onDeleted,
}: {
  tableId: string;
  table: TableInfo;
  recordId: string | null;
  onClose: () => void;
  onSaved: (id: string) => void;
  onDeleted: () => void;
}) {
  const [record, setRecord] = useState<RecordDetail | null>(null);
  const [draft, setDraft] = useState<Draft>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [message, setMessage] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [approval, setApproval] = useState(false);

  useEffect(() => {
    setDraft({});
    setErrors({});
    setMessage(null);
    setApproval(false);
    if (recordId) api.record(tableId, recordId).then(setRecord).catch((e: ApiError) => setMessage(e.message));
    else setRecord(null);
  }, [tableId, recordId]);

  const creating = recordId === null;
  const fields: Column[] = creating ? table.columns.filter((c) => !c.generated) : record?.fields ?? [];

  const save = async () => {
    setSaving(true);
    setErrors({});
    setMessage(null);
    try {
      const saved = creating ? await api.create(tableId, draft) : await api.update(tableId, recordId!, draft);
      setDraft({});
      setRecord(saved);
      setMessage("Saved.");
      onSaved(saved.id);
    } catch (e) {
      const error = e as ApiError;
      const issues = Array.isArray(error.details) ? (error.details as { field: string; message: string }[]) : [];
      const byField: Record<string, string> = {};
      for (const issue of issues) byField[issue.field] = issue.message;
      setErrors(byField);
      setMessage(error.message);
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (!recordId || !window.confirm("Delete this record?")) return;
    try {
      await api.remove(tableId, recordId);
      onDeleted();
    } catch (e) {
      setMessage((e as ApiError).message);
    }
  };

  const dirty = Object.keys(draft).length > 0;
  const canEdit = creating || record?.actions.update?.allowed;

  return (
    <aside className="panel" data-testid="record-panel">
      <div className="panel-head">
        <div>
          <div className="muted small">{table.name}</div>
          <h2 data-testid="panel-title">{creating ? "New record" : record?.title ?? "…"}</h2>
        </div>
        <button className="ghost" onClick={onClose} aria-label="Close" data-testid="close-panel">✕</button>
      </div>
      <div className="panel-body">
        {fields.map((column) => {
          const editable = canEdit && column.editable && !column.generated && column.visibility === "visible";
          return (
            <div className="field-row" key={column.id}>
              <div className="field-label">
                {column.computed && <span className="fx">ƒ</span>}
                {column.name}
                {column.required && <span className="req">*</span>}
                <span className="type-tag">{column.typeLabel}</span>
              </div>
              <div className="field-value">
                {editable ? (
                  <Editor tableId={tableId} column={column} draft={draft} setDraft={setDraft} error={errors[column.id]} />
                ) : (
                  <span className="read-only" data-testid={`value-${column.name}`}>
                    <CellView column={column} cell={column.cell} />
                  </span>
                )}
                {column.formula && <div className="formula small">= {column.formula}</div>}
                {errors[column.id] && <div className="field-error" data-testid="field-error">{errors[column.id]}</div>}
              </div>
            </div>
          );
        })}
        {message && <div className={`notice ${Object.keys(errors).length || message !== "Saved." ? "refused" : "ok"}`} data-testid="panel-message">{message}</div>}
        {canEdit && (
          <div className="panel-actions">
            <button onClick={save} disabled={saving || (!creating && !dirty)} data-testid="save">
              {creating ? "Create" : "Save changes"}
            </button>
            {!creating && record?.actions.delete?.allowed && (
              <button className="danger" onClick={remove} data-testid="delete">Delete</button>
            )}
          </div>
        )}
        {!creating && record && (
          <div className="panel-section">
            <h3>Access</h3>
            <Decision label="Change" explanation={record.actions.update} />
            <Decision label="Delete" explanation={record.actions.delete} />
            {record.actions.approve && (
              <>
                <button className="secondary" onClick={() => setApproval(true)} data-testid="approve">Approve…</button>
                {approval && (
                  <Decision
                    label="Approve"
                    explanation={record.actions.approve}
                    note="The approval workflow itself arrives with mt-lifecycle; this check already uses the real access rules (limits, separation of duties)."
                  />
                )}
              </>
            )}
            <div className="muted small meta">
              Created by {record.createdBy} · last changed by {record.modifiedBy} · version {record.version}
            </div>
          </div>
        )}
      </div>
    </aside>
  );
}
