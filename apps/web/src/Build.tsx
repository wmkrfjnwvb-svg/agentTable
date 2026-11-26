import { useEffect, useState } from "react";
import { api, ApiError, BaseSummary, Column, Person, Setting, Template } from "./api";
import { ConditionEditor, FieldInfo, When } from "./Designer";

// Building: the pages where people assemble their own configuration from the components —
// bases and tables, the template gallery, people and groups, settings, and the field editor.
// Every rule is enforced by the server; these screens only collect what to build.

function useMessage(): [string | null, (e: unknown) => void, () => void] {
  const [message, setMessage] = useState<string | null>(null);
  return [message, (e: unknown) => setMessage((e as ApiError).message ?? String(e)), () => setMessage(null)];
}

// ------------------------------------------------------------------- home

export function HomePage({ bases, canCreate, onChanged }: { bases: BaseSummary[]; canCreate: boolean; onChanged: () => void }) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [message, fail, clear] = useMessage();
  const create = async () => {
    clear();
    try {
      const { id } = await api.createBase(name, description);
      setName("");
      setDescription("");
      onChanged();
      window.location.hash = `#/base/${id}`;
    } catch (e) {
      fail(e);
    }
  };
  return (
    <div className="page" data-testid="home">
      <div className="page-head">
        <div>
          <h1>Home</h1>
          <p className="muted">Your bases, a fresh start, or a template to begin from.</p>
        </div>
      </div>
      {bases.length > 0 && (
        <div className="cards" data-testid="base-cards">
          {bases.map((b) => (
            <a className="card link" key={b.id} href={`#/base/${b.id}`} data-testid="base-card">
              <div className="card-title">◫ {b.name}</div>
              <div className="muted small">{b.description || "—"}</div>
              <div className="big-number">{b.tables.length}</div>
              <div className="muted small">table(s)</div>
            </a>
          ))}
        </div>
      )}
      {bases.length === 0 && (
        <div className="notice soon" data-testid="empty-workspace">
          This workspace is empty. Build your first base from scratch, or install a template below.
        </div>
      )}
      <div className="two-columns">
        {canCreate && (
          <div className="card">
            <h3>Start from scratch</h3>
            <label>
              Base name
              <input value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Expenses" data-testid="new-base-name" />
            </label>
            <label>
              What it is for
              <input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="optional" data-testid="new-base-description" />
            </label>
            <div className="panel-actions">
              <button onClick={create} disabled={!name.trim()} data-testid="create-base">Create base</button>
            </div>
            {message && <div className="notice refused" data-testid="home-message">{message}</div>}
          </div>
        )}
        <TemplateGallery onInstalled={onChanged} />
      </div>
    </div>
  );
}

function TemplateGallery({ onInstalled }: { onInstalled: () => void }) {
  const [templates, setTemplates] = useState<Template[]>([]);
  const [message, fail, clear] = useMessage();
  const [busy, setBusy] = useState(false);
  const load = () => api.templates().then(setTemplates).catch(() => setTemplates([]));
  useEffect(() => {
    load();
  }, []);
  const install = async (key: string) => {
    clear();
    setBusy(true);
    try {
      await api.installTemplate(key);
      await load();
      onInstalled();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="card" data-testid="templates">
      <h3>Templates</h3>
      {templates.map((t) => (
        <div key={t.key} className="template" data-testid={`template-${t.key}`}>
          <div>
            <b>{t.name}</b>
            <div className="muted small">{t.description}</div>
          </div>
          {t.installed ? (
            <span className="badge live">installed</span>
          ) : (
            <button className="secondary" onClick={() => install(t.key)} disabled={busy} data-testid={`install-${t.key}`}>
              Install
            </button>
          )}
        </div>
      ))}
      <p className="muted small">More templates (recruitment, expenses, assets …) arrive with the template engine in wave 6.</p>
      {message && <div className="notice refused">{message}</div>}
    </div>
  );
}

// ------------------------------------------------------------------- a base

export function BasePage({ base, onChanged }: { base: BaseSummary; onChanged: () => void }) {
  const [name, setName] = useState("");
  const [primary, setPrimary] = useState("Name");
  const [message, fail, clear] = useMessage();
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState(base.name);
  useEffect(() => setTitle(base.name), [base.name]);
  const createTable = async () => {
    clear();
    try {
      const table = await api.createTable(base.id, name, primary);
      setName("");
      setPrimary("Name");
      onChanged();
      window.location.hash = `#/table/${table.id}`;
    } catch (e) {
      fail(e);
    }
  };
  const rename = async () => {
    clear();
    try {
      await api.updateBase(base.id, { name: title });
      setRenaming(false);
      onChanged();
    } catch (e) {
      fail(e);
    }
  };
  const removeTable = async (id: string, tableName: string) => {
    if (!window.confirm(`Delete the table “${tableName}” and all its records?`)) return;
    clear();
    try {
      await api.deleteTable(id);
      onChanged();
    } catch (e) {
      fail(e);
    }
  };
  const removeBase = async () => {
    if (!window.confirm(`Delete the base “${base.name}”?`)) return;
    clear();
    try {
      await api.deleteBase(base.id);
      onChanged();
      window.location.hash = "#/";
    } catch (e) {
      fail(e);
    }
  };
  return (
    <div className="page" data-testid="base-page">
      <div className="page-head">
        <div>
          {renaming ? (
            <div className="row">
              <input value={title} onChange={(e) => setTitle(e.target.value)} data-testid="base-title-input" />
              <button onClick={rename} data-testid="base-title-save">Save</button>
              <button className="secondary" onClick={() => setRenaming(false)}>Cancel</button>
            </div>
          ) : (
            <h1 data-testid="base-title">◫ {base.name}</h1>
          )}
          <p className="muted">{base.description || "A base: tables that belong together, with their processes and access."}</p>
        </div>
        {base.canManageSchema && !renaming && (
          <div className="actions">
            <button className="secondary" onClick={() => setRenaming(true)} data-testid="rename-base">Rename</button>
            <button className="danger" onClick={removeBase} data-testid="delete-base">Delete base</button>
          </div>
        )}
      </div>
      <div className="cards">
        {base.tables.map((t) => (
          <div className="card" key={t.id} data-testid="table-card">
            <a href={`#/table/${t.id}`} className="card-title">{t.canRead ? "▦" : "🔒"} {t.name}</a>
            <div className="big-number">{t.canRead ? t.records : "—"}</div>
            <div className="muted small">{t.canRead ? "records you can see" : "no access"}</div>
            {base.canManageSchema && (
              <button className="ghost small" onClick={() => removeTable(t.id, t.name)} data-testid={`delete-table-${t.name}`}>Delete table</button>
            )}
          </div>
        ))}
        {base.tables.length === 0 && <div className="muted empty">No tables yet.</div>}
      </div>
      {base.canManageSchema && (
        <div className="card narrow">
          <h3>New table</h3>
          <label>
            Table name
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Expense claims" data-testid="new-table-name" />
          </label>
          <label>
            Its records are named by
            <input value={primary} onChange={(e) => setPrimary(e.target.value)} placeholder="Name" data-testid="new-table-primary" />
          </label>
          <div className="panel-actions">
            <button onClick={createTable} disabled={!name.trim()} data-testid="create-table">Create table</button>
          </div>
        </div>
      )}
      {message && <div className="notice refused" data-testid="base-message">{message}</div>}
    </div>
  );
}

// ------------------------------------------------------------------- people

const KINDS = [
  ["member", "Member"],
  ["guest", "Guest"],
  ["agent", "AI agent"],
  ["group", "Group"],
];

export function PeoplePage({ people, onChanged }: { people: Person[]; onChanged: () => void }) {
  const [name, setName] = useState("");
  const [title, setTitle] = useState("");
  const [kind, setKind] = useState("member");
  const [message, fail, clear] = useMessage();
  const groups = people.filter((p) => p.kind === "group");
  const individuals = people.filter((p) => p.kind !== "group");
  const add = async () => {
    clear();
    try {
      await api.createPerson(name, kind, title);
      setName("");
      setTitle("");
      onChanged();
    } catch (e) {
      fail(e);
    }
  };
  const toggle = async (group: Person, person: Person) => {
    clear();
    const current = individuals.filter((p) => group.members.includes(p.name)).map((p) => p.id);
    const next = current.includes(person.id) ? current.filter((id) => id !== person.id) : [...current, person.id];
    try {
      await api.updatePerson(group.id, { members: next });
      onChanged();
    } catch (e) {
      fail(e);
    }
  };
  return (
    <div className="page" data-testid="people-page">
      <div className="page-head">
        <div>
          <h1>People & groups</h1>
          <p className="muted">Who works here: members, guests, AI agents, and groups to give access to. Sign-in arrives with mt-identity (wave 4); until then, switch identity at the top right.</p>
        </div>
      </div>
      <div className="two-columns">
        <div className="card">
          <table className="plain" data-testid="people">
            <thead><tr><th>Name</th><th>Kind</th><th>Title</th></tr></thead>
            <tbody>
              {individuals.map((p) => (
                <tr key={p.id}><td>{p.name}</td><td>{p.kind}</td><td className="muted">{p.title}</td></tr>
              ))}
            </tbody>
          </table>
          <h3>Add someone</h3>
          <div className="form-row">
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Name" data-testid="person-name" />
            <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title or role (optional)" data-testid="person-title" />
            <select value={kind} onChange={(e) => setKind(e.target.value)} data-testid="person-kind">
              {KINDS.map(([k, label]) => <option key={k} value={k}>{label}</option>)}
            </select>
            <button onClick={add} disabled={!name.trim()} data-testid="add-person">Add</button>
          </div>
          {message && <div className="notice refused" data-testid="people-message">{message}</div>}
        </div>
        <div className="card">
          <h3>Groups</h3>
          {groups.length === 0 && <p className="muted small">No groups yet: add one with the kind “Group”.</p>}
          {groups.map((g) => (
            <div key={g.id} className="group-card" data-testid={`group-${g.name}`}>
              <b>{g.name}</b>
              <div className="chips wrap">
                {individuals.filter((p) => p.kind !== "agent").map((p) => (
                  <label key={p.id} className={`chip-toggle ${g.members.includes(p.name) ? "on" : ""}`} data-testid={`member-${g.name}-${p.name}`}>
                    <input type="checkbox" checked={g.members.includes(p.name)} onChange={() => toggle(g, p)} />
                    {p.name}
                  </label>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ------------------------------------------------------------------- settings

export function SettingsPage() {
  const [settings, setSettings] = useState<Setting[]>([]);
  const [bases, setBases] = useState<BaseSummary[]>([]);
  const [key, setKey] = useState("");
  const [type, setType] = useState("money");
  const [text, setText] = useState("");
  const [base, setBase] = useState("");
  const [chosen, setChosen] = useState("");
  const [explainFor, setExplainFor] = useState("");
  const [explanation, setExplanation] = useState("");
  const [message, fail, clear] = useMessage();
  const load = () => api.settings().then(setSettings).catch(() => setSettings([]));
  useEffect(() => {
    load();
    api.bases().then((r) => setBases(r.bases)).catch(() => setBases([]));
  }, []);
  useEffect(() => {
    if (!chosen) return;
    api.explainSetting(chosen, explainFor).then((r) => setExplanation(r.text)).catch((e: ApiError) => setExplanation(e.message));
  }, [chosen, explainFor, settings]);
  const save = async () => {
    clear();
    try {
      await api.setSetting(key, type, text, base);
      setChosen(key.trim());
      setText("");
      load();
    } catch (e) {
      fail(e);
    }
  };
  const remove = async (k: string, b: string) => {
    clear();
    try {
      await api.clearSetting(k, b);
      load();
    } catch (e) {
      fail(e);
    }
  };
  const selected = settings.find((s) => s.key === chosen);
  return (
    <div className="page" data-testid="settings-page">
      <div className="page-head">
        <div>
          <h1>Settings</h1>
          <p className="muted">
            Named values your formulas read with <code>PARAM("key")</code>: limits, rates, thresholds. Set one for the whole workspace and override it for a base: a formula
            uses the value of its record's base. Change one and every formula that reads it recomputes; every change is kept.
          </p>
        </div>
      </div>
      <div className="card">
        <table className="plain" data-testid="settings">
          <thead><tr><th>Key</th><th>Type</th><th>Whole workspace</th><th>Per base</th></tr></thead>
          <tbody>
            {settings.map((s) => (
              <tr key={s.key} onClick={() => { setChosen(s.key); setKey(s.key); setType(s.type); setText(s.value.replace(/,/g, "")); }} className={`clickable ${chosen === s.key ? "selected" : ""}`} data-testid={`setting-${s.key}`}>
                <td><code>{s.key}</code></td>
                <td>{s.type}</td>
                <td>{s.value || <span className="muted">—</span>}</td>
                <td>
                  {s.overrides.filter((o) => o.base).map((o) => (
                    <span key={o.base} className="chip override" data-testid={`override-${s.key}-${o.for}`}>
                      {o.for.replace(/^the base /, "")}: {o.value}
                      <button className="ghost small" title="Remove: the workspace value applies again" onClick={(e) => { e.stopPropagation(); remove(s.key, o.base ?? ""); }} data-testid={`clear-${s.key}-${o.for}`}>✕</button>
                    </span>
                  ))}
                </td>
              </tr>
            ))}
            {settings.length === 0 && <tr><td colSpan={4} className="muted">No settings yet.</td></tr>}
          </tbody>
        </table>
        <h3>Set a value</h3>
        <div className="form-row">
          <input value={key} onChange={(e) => setKey(e.target.value)} placeholder="expense.manager_limit" data-testid="setting-key" />
          <select value={type} onChange={(e) => setType(e.target.value)} data-testid="setting-type">
            {["money", "number", "percent", "text", "bool"].map((t) => <option key={t} value={t}>{t}</option>)}
          </select>
          <input value={text} onChange={(e) => setText(e.target.value)} placeholder={type === "money" ? "1000 USD" : type === "percent" ? "8%" : ""} data-testid="setting-value" />
          <span>for</span>
          <select value={base} onChange={(e) => setBase(e.target.value)} data-testid="setting-for">
            <option value="">the whole workspace</option>
            {bases.map((b) => <option key={b.id} value={b.id}>the base {b.name}</option>)}
          </select>
          <button onClick={save} disabled={!key.trim() || !text.trim()} data-testid="save-setting">Save</button>
        </div>
        {message && <div className="notice refused" data-testid="settings-message">{message}</div>}
      </div>
      {selected && (
        <div className="card" data-testid="setting-detail">
          <h3><code>{selected.key}</code></h3>
          <div className="form-row">
            <span>Value for</span>
            <select value={explainFor} onChange={(e) => setExplainFor(e.target.value)} data-testid="explain-for">
              <option value="">the whole workspace</option>
              {bases.map((b) => <option key={b.id} value={b.id}>the base {b.name}</option>)}
            </select>
          </div>
          <div className="notice" data-testid="setting-explain">{explanation}</div>
          <h4>History</h4>
          <ul className="versions" data-testid="setting-history">
            {selected.history.map((h, i) => (
              <li key={i}>
                <b>{h.value}</b> for {h.for} <span className="muted small">· {h.by} · {new Date(h.at).toISOString().slice(0, 16).replace("T", " ")}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

// ------------------------------------------------------------------- the field editor

export function FieldEditor({ tableId, column, columns, onClose, onSaved }: { tableId: string; column: Column; columns: Column[]; onClose: () => void; onSaved: () => void }) {
  const [name, setName] = useState(column.name);
  const [required, setRequired] = useState(column.required);
  const [when, setWhen] = useState<When | null>(column.requiredWhen ?? null);
  const others: FieldInfo[] = columns
    .filter((c) => c.id !== column.id)
    .map((c) => ({ id: c.id, name: c.name, dataType: c.dataType, type: c.type, generated: c.generated, options: c.options ? c.options.filter((o) => o.enabled) : null }));
  const whenChanged = JSON.stringify(when) !== JSON.stringify(column.requiredWhen ?? null);
  const [help, setHelp] = useState(column.help ?? "");
  const [choices, setChoices] = useState(
    (column.options ?? []).filter((o) => o.enabled).map((o) => ({ id: o.id as string | undefined, label: o.label })),
  );
  const [message, fail, clear] = useMessage();
  const save = async () => {
    clear();
    try {
      await api.updateField(tableId, column.id, {
        name: name !== column.name ? name : undefined,
        required: required !== column.required ? required : undefined,
        help: help !== (column.help ?? "") ? help : undefined,
        options: column.options ? choices.filter((c) => c.label.trim()) : undefined,
        requiredWhen: whenChanged ? when : undefined,
      });
      onSaved();
    } catch (e) {
      fail(e);
    }
  };
  const remove = async () => {
    if (!window.confirm(`Delete the field “${column.name}” and its values?`)) return;
    clear();
    try {
      await api.deleteField(tableId, column.id);
      onSaved();
    } catch (e) {
      fail(e);
    }
  };
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal small-modal" onClick={(e) => e.stopPropagation()} data-testid="field-editor">
        <div className="modal-head">
          <h2>{column.typeLabel} field</h2>
          <button className="ghost" onClick={onClose} aria-label="Close">✕</button>
        </div>
        <div className="modal-body">
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} data-testid="edit-field-name" />
          </label>
          {!column.generated && (
            <label className="inline">
              <input type="checkbox" checked={required} onChange={(e) => setRequired(e.target.checked)} data-testid="edit-field-required" /> Required
            </label>
          )}
          {!column.generated && !required && (
            <div className="row wrap" data-testid="required-when">
              <span className="small muted">Required</span>
              <ConditionEditor when={when} fields={others} onChange={setWhen} testId="required-when" />
            </div>
          )}
          <label>
            Help text
            <input value={help} onChange={(e) => setHelp(e.target.value)} placeholder="Shown next to the field" data-testid="edit-field-help" />
          </label>
          {column.formula && <div className="small">Formula: <code>{column.formula}</code></div>}
          {column.options && (
            <div>
              <div className="small muted">Choices (removed ones are disabled; records keep them)</div>
              {choices.map((c, i) => (
                <div className="row" key={i}>
                  <input value={c.label} onChange={(e) => setChoices(choices.map((x, j) => (j === i ? { ...x, label: e.target.value } : x)))} data-testid="edit-choice" />
                  <button className="ghost" onClick={() => setChoices(choices.filter((_, j) => j !== i))} title="Remove">✕</button>
                </div>
              ))}
              <button className="secondary small" onClick={() => setChoices([...choices, { id: undefined, label: "" }])} data-testid="add-choice">+ Choice</button>
            </div>
          )}
          {message && <div className="notice refused" data-testid="field-editor-message">{message}</div>}
          <div className="panel-actions">
            <button onClick={save} data-testid="save-field">Save</button>
            {!column.primary && <button className="danger" onClick={remove} data-testid="delete-field">Delete field</button>}
            <button className="secondary" onClick={onClose}>Cancel</button>
          </div>
        </div>
      </div>
    </div>
  );
}
