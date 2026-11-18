import { useEffect, useMemo, useState } from "react";
import { api, ApiError, TableSummary } from "./api";

type FieldType = { key: string; label: string; group: string; synonyms: string[]; requirements: string[]; unavailable: string | null };
type Check = Awaited<ReturnType<typeof api.checkFormula>>;

const GROUP_LABEL: Record<string, string> = {
  basic: "Basic",
  people: "People & organisation",
  business: "Business",
  reference: "Reference",
  relational: "Relational & computed",
  process: "Process",
  ai: "AI",
  system: "System",
};

const TAX_FORMATS = [
  ["tax.de_vat", "German VAT ID"],
  ["tax.us_ein", "US EIN"],
  ["tax.jp_corporate", "Japanese corporate number"],
  ["tax.jp_invoice", "Japanese invoice number"],
  ["tax.kr_brn", "Korean business number"],
  ["tax.in_gstin", "Indian GSTIN"],
  ["tax.in_pan", "Indian PAN"],
];

export default function FieldPicker({ tableId, onClose, onCreated }: { tableId: string; onClose: () => void; onCreated: () => void }) {
  const [groups, setGroups] = useState<{ key: string; types: FieldType[] }[]>([]);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<string[] | null>(null);
  const [chosen, setChosen] = useState<FieldType | null>(null);
  const [name, setName] = useState("");
  const [options, setOptions] = useState("");
  const [target, setTarget] = useState("");
  const [refset, setRefset] = useState("");
  const [formula, setFormula] = useState("");
  const [pattern, setPattern] = useState("{seq:4}");
  const [format, setFormat] = useState("tax.de_vat");
  const [check, setCheck] = useState<Check | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tables, setTables] = useState<TableSummary[]>([]);
  const [sets, setSets] = useState<{ id: string; name: string }[]>([]);

  useEffect(() => {
    api.fieldTypes().then((r) => setGroups(r.groups));
    api.tables().then(setTables);
    api.refsets().then((list) => setSets(list.map((s) => ({ id: s.id, name: s.name }))));
  }, []);

  useEffect(() => {
    if (!query.trim()) {
      setFound(null);
      return;
    }
    const timer = window.setTimeout(() => api.fieldTypes(query).then((r) => setFound(r.search)), 150);
    return () => window.clearTimeout(timer);
  }, [query]);

  const needsFormula = chosen?.requirements.includes("formula");
  useEffect(() => {
    if (!needsFormula || !formula.trim()) {
      setCheck(null);
      return;
    }
    const timer = window.setTimeout(() => api.checkFormula(tableId, formula).then(setCheck).catch(() => setCheck(null)), 250);
    return () => window.clearTimeout(timer);
  }, [formula, needsFormula, tableId]);

  const all = useMemo(() => groups.flatMap((g) => g.types), [groups]);
  const visibleGroups = found
    ? [{ key: "results", types: found.map((k) => all.find((t) => t.key === k)).filter((t): t is FieldType => !!t) }]
    : groups;

  const create = async () => {
    if (!chosen) return;
    setError(null);
    try {
      await api.addField(tableId, {
        type: chosen.key,
        name,
        options: options.split(",").map((s) => s.trim()).filter(Boolean),
        target: target || undefined,
        refset: refset || undefined,
        formula: formula || undefined,
        pattern,
        format,
      });
      onCreated();
    } catch (e) {
      setError((e as ApiError).message);
    }
  };

  const ready = chosen && !chosen.unavailable && name.trim() && (!needsFormula || check?.ok);

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()} data-testid="field-picker">
        <div className="modal-head">
          <h2>Add a field</h2>
          <button className="ghost" onClick={onClose} aria-label="Close">✕</button>
        </div>
        <div className="picker">
          <div className="picker-types">
            <input className="search" placeholder="Search types (try “dropdown”, “status”, “money”)" value={query} onChange={(e) => setQuery(e.target.value)} data-testid="type-search" autoFocus />
            {visibleGroups.map((g) => (
              <div key={g.key} className="type-group">
                <div className="type-group-title">{g.key === "results" ? "Matches" : GROUP_LABEL[g.key] ?? g.key}</div>
                <div className="type-grid">
                  {g.types.map((t) => (
                    <button
                      key={t.key}
                      className={`type ${chosen?.key === t.key ? "chosen" : ""} ${t.unavailable ? "unavailable" : ""}`}
                      title={t.unavailable ?? t.synonyms.join(", ")}
                      onClick={() => { setChosen(t); if (!name) setName(""); }}
                      data-testid={`type-${t.key}`}
                    >
                      {t.label}
                      {t.unavailable && <span className="badge soon">soon</span>}
                    </button>
                  ))}
                  {g.types.length === 0 && <span className="muted small">No type matches.</span>}
                </div>
              </div>
            ))}
          </div>
          <div className="picker-form">
            {!chosen && <p className="muted">Choose a type on the left.</p>}
            {chosen && (
              <>
                <h3>{chosen.label}</h3>
                {chosen.unavailable && <div className="notice soon">Not available yet: {chosen.unavailable}.</div>}
                <label>
                  Name
                  <input value={name} onChange={(e) => setName(e.target.value)} data-testid="field-name" />
                </label>
                {chosen.requirements.includes("options") && (
                  <label>
                    Options (comma-separated)
                    <input value={options} onChange={(e) => setOptions(e.target.value)} placeholder="Low, Medium, High" data-testid="field-options" />
                  </label>
                )}
                {chosen.requirements.includes("link_target") && (
                  <label>
                    Links to
                    <select value={target} onChange={(e) => setTarget(e.target.value)} data-testid="field-target">
                      <option value="">Choose a table…</option>
                      {tables.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
                    </select>
                  </label>
                )}
                {chosen.requirements.includes("ref_set") && (
                  <label>
                    Values from
                    <select value={refset} onChange={(e) => setRefset(e.target.value)} data-testid="field-refset">
                      <option value="">Choose a reference set…</option>
                      {sets.map((s) => <option key={s.id} value={s.id}>{s.name}</option>)}
                    </select>
                  </label>
                )}
                {chosen.requirements.includes("numbering") && (
                  <label>
                    Pattern
                    <input value={pattern} onChange={(e) => setPattern(e.target.value)} placeholder="INV-{YYYY}-{seq:5}" />
                  </label>
                )}
                {chosen.requirements.includes("format") && (
                  <label>
                    Format
                    <select value={format} onChange={(e) => setFormat(e.target.value)}>
                      {TAX_FORMATS.map(([k, label]) => <option key={k} value={k}>{label}</option>)}
                    </select>
                  </label>
                )}
                {needsFormula && (
                  <div className="formula-editor">
                    <label>
                      Formula
                      <textarea
                        value={formula}
                        onChange={(e) => setFormula(e.target.value)}
                        rows={3}
                        spellCheck={false}
                        placeholder={chosen.key === "lookup" ? "{Lines}.{Item}" : chosen.key === "rollup" ? "SUM({Lines}.{Amount})" : "{Total} - {Tax}"}
                        data-testid="formula"
                      />
                    </label>
                    {check && !check.ok && (
                      <div className="notice refused" data-testid="formula-error">
                        {check.error}
                        {typeof check.position === "number" && (
                          <pre className="caret">{formula}{"\n"}{" ".repeat(check.position)}^</pre>
                        )}
                      </div>
                    )}
                    {check?.ok && (
                      <div className="notice ok" data-testid="formula-ok">
                        <div>Result: <b>{check.type}</b> · reads {check.reads?.join(", ") || "no field"}</div>
                        <div className="small">Normalised: <code>{check.normalized}</code></div>
                        {check.warnings?.map((w) => <div className="small warn" key={w}>⚠ {w}</div>)}
                        {check.preview && check.preview.length > 0 && (
                          <table className="preview">
                            <tbody>
                              {check.preview.map((p, i) => (
                                <tr key={i}><td>{p.record}</td><td className="num" data-testid="preview-value">{p.value}</td></tr>
                              ))}
                            </tbody>
                          </table>
                        )}
                      </div>
                    )}
                  </div>
                )}
                {error && <div className="notice refused" data-testid="field-error">{error}</div>}
                <div className="panel-actions">
                  <button disabled={!ready} onClick={create} data-testid="create-field">Create field</button>
                  <button className="secondary" onClick={onClose}>Cancel</button>
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
