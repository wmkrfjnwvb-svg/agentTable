import { useEffect, useState } from "react";
import { api, ApiError, Person } from "./api";

// The process designer: a table's lifecycle (states, buttons, locks, required fields) and its
// approval (steps, approvers, conditions, outcomes). The server compiles the design into the
// engines and refuses designs that could strand a record — the reasons are shown here.

type Approver = { kind: "person"; id: string } | { kind: "field"; field: string } | { kind: "capability"; capability: string };
export type When = { field: string; op: string; text: string };
type StateRow = { uid: string; key: string; name: string; category: string; required: string[]; locked: boolean; editable: string[] };
type ButtonRow = { uid: string; key: string; name: string; from: string[]; to: string; capability: string; when: When | null };
type StepRow = { uid: string; kind: "approval" | "notify"; name: string; approvers: Approver[]; mode: string; canSendBack: boolean; when: When | null };
export type FieldInfo = { id: string; name: string; dataType: string; type: string; generated: boolean; options: { id: string; label: string }[] | null };

type Design = {
  name: string;
  states: { key: string; name: string; category: string; required: string[]; locked: boolean; editable: string[] }[];
  initial: string;
  transitions: { key: string; name: string; from: string[]; to: string; capability: string | null; when: When | null }[];
  approval: {
    name: string;
    startedBy: string;
    approved: string;
    rejected: string;
    returned: string;
    steps: ({ kind: "approval"; name: string; approvers: Approver[]; mode: { mode: string }; canSendBack: boolean; when: When | null } | { kind: "notify"; name: string; to: Approver[]; when: When | null })[];
  } | null;
};

const CATEGORIES: [string, string][] = [
  ["new", "New"],
  ["in_progress", "In progress"],
  ["waiting_internal", "Waiting (internal)"],
  ["waiting_external", "Waiting (external, SLA paused)"],
  ["pending_approval", "Pending approval"],
  ["done_success", "Done"],
  ["done_failure", "Failed / rejected"],
  ["cancelled", "Cancelled"],
];

const OPS: Record<string, [string, string][]> = {
  bool: [["eq", "is"]],
  number: [["gt", ">"], ["ge", "≥"], ["lt", "<"], ["le", "≤"], ["eq", "="]],
  money: [["gt", ">"], ["ge", "≥"], ["lt", "<"], ["le", "≤"], ["eq", "="]],
  percent: [["gt", ">"], ["ge", "≥"], ["lt", "<"], ["le", "≤"]],
  reference: [["eq", "is"], ["ne", "is not"]],
  text: [["eq", "is"], ["contains", "contains"], ["is_not_empty", "is filled in"], ["is_empty", "is empty"]],
};

let counter = 0;
const uid = () => `u${++counter}`;

function slug(name: string): string {
  const s = name.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 40);
  return /^[a-z]/.test(s) ? s : `s_${s}`;
}

function starter(): { states: StateRow[]; buttons: ButtonRow[]; initial: string } {
  const draft = { uid: uid(), key: "", name: "Draft", category: "new", required: [], locked: false, editable: [] };
  const done = { uid: uid(), key: "", name: "Done", category: "done_success", required: [], locked: false, editable: [] };
  return {
    states: [draft, done],
    buttons: [{ uid: uid(), key: "", name: "Complete", from: [draft.uid], to: done.uid, capability: "", when: null }],
    initial: draft.uid,
  };
}

export function ConditionEditor({ when, fields, onChange, testId }: { when: When | null; fields: FieldInfo[]; onChange: (w: When | null) => void; testId: string }) {
  const usable = fields.filter((f) => ["bool", "number", "money", "percent", "reference", "text"].includes(f.dataType));
  if (!when) {
    return (
      <button className="ghost small" onClick={() => usable[0] && onChange({ field: usable[0].id, op: OPS[usable[0].dataType][0][0], text: "" })} data-testid={`${testId}-add`}>
        + only when…
      </button>
    );
  }
  const field = fields.find((f) => f.id === when.field);
  const ops = OPS[field?.dataType ?? "text"] ?? OPS.text;
  return (
    <span className="condition" data-testid={testId}>
      only when
      <select value={when.field} onChange={(e) => { const f = fields.find((x) => x.id === e.target.value); onChange({ field: e.target.value, op: OPS[f?.dataType ?? "text"][0][0], text: "" }); }} data-testid={`${testId}-field`}>
        {usable.map((f) => <option key={f.id} value={f.id}>{f.name}</option>)}
      </select>
      <select value={when.op} onChange={(e) => onChange({ ...when, op: e.target.value })} data-testid={`${testId}-op`}>
        {ops.map(([k, label]) => <option key={k} value={k}>{label}</option>)}
      </select>
      {!["is_empty", "is_not_empty"].includes(when.op) && (
        field?.dataType === "bool" ? (
          <select value={when.text} onChange={(e) => onChange({ ...when, text: e.target.value })} data-testid={`${testId}-value`}>
            <option value="">…</option>
            <option value="yes">Yes</option>
            <option value="no">No</option>
          </select>
        ) : field?.dataType === "reference" && field.options ? (
          <select value={when.text} onChange={(e) => onChange({ ...when, text: e.target.value })} data-testid={`${testId}-value`}>
            <option value="">…</option>
            {field.options.map((o) => <option key={o.id} value={o.id}>{o.label}</option>)}
          </select>
        ) : (
          <input value={when.text} onChange={(e) => onChange({ ...when, text: e.target.value })} placeholder={field?.dataType === "money" ? "1000 USD" : "value"} data-testid={`${testId}-value`} />
        )
      )}
      <button className="ghost small" onClick={() => onChange(null)} title="Remove the condition">✕</button>
    </span>
  );
}

function ApproverEditor({ approver, people, fields, onChange, onRemove, testId }: { approver: Approver; people: Person[]; fields: FieldInfo[]; onChange: (a: Approver) => void; onRemove: () => void; testId: string }) {
  const personFields = fields.filter((f) => f.dataType === "principal");
  return (
    <span className="approver" data-testid={testId}>
      <select
        value={approver.kind}
        onChange={(e) => {
          const kind = e.target.value;
          if (kind === "person") onChange({ kind: "person", id: people[0]?.id ?? "" });
          else if (kind === "field") onChange({ kind: "field", field: personFields[0]?.id ?? "" });
          else onChange({ kind: "capability", capability: "" });
        }}
        data-testid={`${testId}-kind`}
      >
        <option value="person">a person</option>
        <option value="field" disabled={personFields.length === 0}>the person in a field</option>
        <option value="capability">whoever holds a capability</option>
      </select>
      {approver.kind === "person" && (
        <select value={approver.id} onChange={(e) => onChange({ kind: "person", id: e.target.value })} data-testid={`${testId}-person`}>
          {people.filter((p) => p.kind === "member" || p.kind === "group").map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
      )}
      {approver.kind === "field" && (
        <select value={approver.field} onChange={(e) => onChange({ kind: "field", field: e.target.value })} data-testid={`${testId}-field`}>
          {personFields.map((f) => <option key={f.id} value={f.id}>{f.name}</option>)}
        </select>
      )}
      {approver.kind === "capability" && (
        <input value={approver.capability} onChange={(e) => onChange({ kind: "capability", capability: e.target.value })} placeholder="e.g. expense.approve" data-testid={`${testId}-capability`} />
      )}
      <button className="ghost small" onClick={onRemove} title="Remove">✕</button>
    </span>
  );
}

export default function DesignerPage({ tableId }: { tableId: string }) {
  const [tableName, setTableName] = useState("");
  const [fields, setFields] = useState<FieldInfo[]>([]);
  const [people, setPeople] = useState<Person[]>([]);
  const [name, setName] = useState("");
  const [states, setStates] = useState<StateRow[]>([]);
  const [buttons, setButtons] = useState<ButtonRow[]>([]);
  const [initial, setInitial] = useState("");
  const [hasApproval, setHasApproval] = useState(false);
  const [approvalName, setApprovalName] = useState("Approval");
  const [startedBy, setStartedBy] = useState("");
  const [approved, setApproved] = useState("");
  const [rejected, setRejected] = useState("");
  const [returned, setReturned] = useState("");
  const [steps, setSteps] = useState<StepRow[]>([]);
  const [published, setPublished] = useState<{ name: string; version: number } | null>(null);
  const [isNew, setIsNew] = useState(true);
  const [canDesign, setCanDesign] = useState(false);
  const [running, setRunning] = useState(0);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  const load = async () => {
    const [info, everyone] = await Promise.all([
      api.processDesign(tableId),
      api.people(),
    ]);
    setPeople(everyone);
    setTableName(info.table.name);
    setFields(info.fields);
    setCanDesign(info.canDesign);
    setRunning(info.running);
    setPublished(info.published);
    const design = info.design as Design | null;
    if (!design) {
      const s = starter();
      setIsNew(true);
      setName(info.table.name);
      setStates(s.states);
      setButtons(s.buttons);
      setInitial(s.initial);
      setHasApproval(false);
      setSteps([]);
      return;
    }
    setIsNew(false);
    const byKey = new Map<string, string>();
    const rows = design.states.map((s) => {
      const row = { uid: uid(), ...s };
      byKey.set(s.key, row.uid);
      return row;
    });
    const buttonRows = design.transitions.map((t) => ({
      uid: uid(),
      key: t.key,
      name: t.name,
      from: t.from.map((k) => byKey.get(k) ?? k),
      to: byKey.get(t.to) ?? t.to,
      capability: t.capability ?? "",
      when: t.when,
    }));
    setName(design.name);
    setStates(rows);
    setButtons(buttonRows);
    setInitial(byKey.get(design.initial) ?? "");
    const a = design.approval;
    setHasApproval(!!a);
    if (a) {
      setApprovalName(a.name);
      setStartedBy(buttonRows.find((b) => b.key === a.startedBy)?.uid ?? "");
      setApproved(byKey.get(a.approved) ?? "");
      setRejected(byKey.get(a.rejected) ?? "");
      setReturned(byKey.get(a.returned) ?? "");
      setSteps(
        a.steps.map((s) =>
          s.kind === "approval"
            ? { uid: uid(), kind: "approval", name: s.name, approvers: s.approvers, mode: s.mode.mode, canSendBack: s.canSendBack, when: s.when }
            : { uid: uid(), kind: "notify", name: s.name, approvers: s.to, mode: "any", canSendBack: false, when: s.when },
        ),
      );
    }
  };

  useEffect(() => {
    load().catch((e: ApiError) => setMessage({ ok: false, text: e.message }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tableId]);

  const editable = fields.filter((f) => !f.generated && f.type !== "lifecycle_state");

  const publish = async () => {
    setMessage(null);
    // Final keys: kept for published states and buttons, derived from names for new ones.
    const taken = new Set<string>();
    const keyOf = new Map<string, string>();
    for (const s of states) {
      let key = s.key || slug(s.name);
      let n = 2;
      while (taken.has(key)) key = `${s.key || slug(s.name)}_${n++}`;
      taken.add(key);
      keyOf.set(s.uid, key);
    }
    const buttonKeys = new Set<string>();
    const buttonKeyOf = new Map<string, string>();
    for (const b of buttons) {
      let key = b.key || slug(b.name);
      let n = 2;
      while (buttonKeys.has(key)) key = `${b.key || slug(b.name)}_${n++}`;
      buttonKeys.add(key);
      buttonKeyOf.set(b.uid, key);
    }
    const design: Design = {
      name,
      states: states.map((s) => ({ key: keyOf.get(s.uid)!, name: s.name, category: s.category, required: s.required, locked: s.locked, editable: s.editable })),
      initial: keyOf.get(initial) ?? "",
      transitions: buttons.map((b) => ({
        key: buttonKeyOf.get(b.uid)!,
        name: b.name,
        from: b.from.map((u) => keyOf.get(u) ?? u),
        to: keyOf.get(b.to) ?? "",
        capability: b.capability.trim() || null,
        when: b.when,
      })),
      approval: hasApproval
        ? {
            name: approvalName,
            startedBy: buttonKeyOf.get(startedBy) ?? "",
            approved: keyOf.get(approved) ?? "",
            rejected: keyOf.get(rejected) ?? "",
            returned: keyOf.get(returned) ?? "",
            steps: steps.map((s) =>
              s.kind === "approval"
                ? { kind: "approval" as const, name: s.name, approvers: s.approvers, mode: { mode: s.mode }, canSendBack: s.canSendBack, when: s.when }
                : { kind: "notify" as const, name: s.name, to: s.approvers, when: s.when },
            ),
          }
        : null,
    };
    try {
      await api.publishProcess(tableId, design);
      await load();
      setMessage({ ok: true, text: "Published. Records follow the new design; running approvals keep the version they started with." });
    } catch (e) {
      setMessage({ ok: false, text: (e as ApiError).message });
    }
  };

  const setState = (i: number, patch: Partial<StateRow>) => setStates(states.map((s, j) => (j === i ? { ...s, ...patch } : s)));
  const setButton = (i: number, patch: Partial<ButtonRow>) => setButtons(buttons.map((b, j) => (j === i ? { ...b, ...patch } : b)));
  const setStep = (i: number, patch: Partial<StepRow>) => setSteps(steps.map((s, j) => (j === i ? { ...s, ...patch } : s)));
  const toggle = (list: string[], value: string) => (list.includes(value) ? list.filter((x) => x !== value) : [...list, value]);
  const stateOptions = states.map((s) => <option key={s.uid} value={s.uid}>{s.name || "(unnamed)"}</option>);

  return (
    <div className="page designer" data-testid="designer">
      <div className="page-head">
        <div>
          <h1>Process of {tableName}</h1>
          <p className="muted">
            {isNew ? "No lifecycle yet: this is a starting point." : "Published."}{" "}
            {published && `Approval “${published.name}” version ${published.version}.`} {running > 0 && `${running} approval(s) running.`}
          </p>
        </div>
        <div className="actions">
          <a className="button secondary" href={`#/table/${tableId}`}>Back to the table</a>
          {canDesign && <button onClick={publish} data-testid="publish-process">Publish</button>}
        </div>
      </div>
      {message && <div className={`notice ${message.ok ? "ok" : "refused"}`} data-testid="designer-message">{message.text}</div>}

      <div className="card">
        <h3>Lifecycle</h3>
        <label className="inline">Name <input value={name} onChange={(e) => setName(e.target.value)} data-testid="lifecycle-name" /></label>
        <table className="plain design-table">
          <thead><tr><th>State</th><th>Kind</th><th>Lock fields</th><th>Needs to enter</th><th /></tr></thead>
          <tbody>
            {states.map((s, i) => (
              <tr key={s.uid} data-testid="state-row">
                <td><input value={s.name} onChange={(e) => setState(i, { name: e.target.value })} data-testid={`state-name-${i}`} /></td>
                <td>
                  <select value={s.category} onChange={(e) => setState(i, { category: e.target.value })} data-testid={`state-category-${i}`}>
                    {CATEGORIES.map(([k, label]) => <option key={k} value={k}>{label}</option>)}
                  </select>
                </td>
                <td><input type="checkbox" checked={s.locked} onChange={(e) => setState(i, { locked: e.target.checked })} data-testid={`state-locked-${i}`} /></td>
                <td>
                  <div className="chips wrap">
                    {editable.map((f) => (
                      <label key={f.id} className={`chip-toggle ${s.required.includes(f.id) ? "on" : ""}`} data-testid={`state-required-${i}-${f.name}`}>
                        <input type="checkbox" checked={s.required.includes(f.id)} onChange={() => setState(i, { required: toggle(s.required, f.id) })} />
                        {f.name}
                      </label>
                    ))}
                  </div>
                </td>
                <td><button className="ghost small" onClick={() => setStates(states.filter((_, j) => j !== i))} data-testid={`remove-state-${i}`}>✕</button></td>
              </tr>
            ))}
          </tbody>
        </table>
        <div className="row">
          <button className="secondary small" onClick={() => setStates([...states, { uid: uid(), key: "", name: "", category: "in_progress", required: [], locked: false, editable: [] }])} data-testid="add-state">+ State</button>
          <label className="inline">New records start as
            <select value={initial} onChange={(e) => setInitial(e.target.value)} data-testid="initial-state">{stateOptions}</select>
          </label>
        </div>
      </div>

      <div className="card">
        <h3>Buttons</h3>
        <p className="muted small">Each button moves a record from some states to one state. Leave “from” empty for “from any open state” (a Cancel button).</p>
        <table className="plain design-table">
          <thead><tr><th>Label</th><th>From</th><th>To</th><th>Who may press it</th><th>Condition</th><th /></tr></thead>
          <tbody>
            {buttons.map((b, i) => (
              <tr key={b.uid} data-testid="button-row">
                <td><input value={b.name} onChange={(e) => setButton(i, { name: e.target.value })} data-testid={`button-name-${i}`} /></td>
                <td>
                  <div className="chips wrap">
                    {states.map((s) => (
                      <label key={s.uid} className={`chip-toggle ${b.from.includes(s.uid) ? "on" : ""}`} data-testid={`button-from-${i}-${s.name}`}>
                        <input type="checkbox" checked={b.from.includes(s.uid)} onChange={() => setButton(i, { from: toggle(b.from, s.uid) })} />
                        {s.name || "(unnamed)"}
                      </label>
                    ))}
                  </div>
                </td>
                <td><select value={b.to} onChange={(e) => setButton(i, { to: e.target.value })} data-testid={`button-to-${i}`}><option value="">…</option>{stateOptions}</select></td>
                <td><input value={b.capability} onChange={(e) => setButton(i, { capability: e.target.value })} placeholder="anyone who may edit it" data-testid={`button-capability-${i}`} /></td>
                <td><ConditionEditor when={b.when} fields={fields} onChange={(when) => setButton(i, { when })} testId={`button-when-${i}`} /></td>
                <td><button className="ghost small" onClick={() => setButtons(buttons.filter((_, j) => j !== i))} data-testid={`remove-button-${i}`}>✕</button></td>
              </tr>
            ))}
          </tbody>
        </table>
        <button className="secondary small" onClick={() => setButtons([...buttons, { uid: uid(), key: "", name: "", from: [], to: "", capability: "", when: null }])} data-testid="add-button">+ Button</button>
      </div>

      <div className="card">
        <h3>Approval</h3>
        <label className="inline"><input type="checkbox" checked={hasApproval} onChange={(e) => setHasApproval(e.target.checked)} data-testid="approval-enabled" /> A button starts an approval</label>
        {hasApproval && (
          <>
            <div className="form-row">
              <label className="inline">Name <input value={approvalName} onChange={(e) => setApprovalName(e.target.value)} data-testid="approval-name" /></label>
              <label className="inline">Started by
                <select value={startedBy} onChange={(e) => setStartedBy(e.target.value)} data-testid="approval-start">
                  <option value="">…</option>
                  {buttons.map((b) => <option key={b.uid} value={b.uid}>{b.name || "(unnamed)"}</option>)}
                </select>
              </label>
            </div>
            <div className="form-row">
              <label className="inline">When approved go to <select value={approved} onChange={(e) => setApproved(e.target.value)} data-testid="approval-approved"><option value="">…</option>{stateOptions}</select></label>
              <label className="inline">rejected <select value={rejected} onChange={(e) => setRejected(e.target.value)} data-testid="approval-rejected"><option value="">…</option>{stateOptions}</select></label>
              <label className="inline">sent back <select value={returned} onChange={(e) => setReturned(e.target.value)} data-testid="approval-returned"><option value="">…</option>{stateOptions}</select></label>
            </div>
            <ol className="steps">
              {steps.map((s, i) => (
                <li key={s.uid} className="step-card" data-testid="step-row">
                  <div className="row">
                    <select value={s.kind} onChange={(e) => setStep(i, { kind: e.target.value as "approval" | "notify" })} data-testid={`step-kind-${i}`}>
                      <option value="approval">Approval</option>
                      <option value="notify">Tell people</option>
                    </select>
                    <input value={s.name} onChange={(e) => setStep(i, { name: e.target.value })} placeholder="Step name, e.g. Manager" data-testid={`step-name-${i}`} />
                    <button className="ghost small" onClick={() => setSteps(steps.filter((_, j) => j !== i))} data-testid={`remove-step-${i}`}>✕</button>
                  </div>
                  <div className="row wrap">
                    {s.approvers.map((a, j) => (
                      <ApproverEditor
                        key={j}
                        approver={a}
                        people={people}
                        fields={fields}
                        onChange={(next) => setStep(i, { approvers: s.approvers.map((x, k) => (k === j ? next : x)) })}
                        onRemove={() => setStep(i, { approvers: s.approvers.filter((_, k) => k !== j) })}
                        testId={`step-${i}-approver-${j}`}
                      />
                    ))}
                    <button className="ghost small" onClick={() => setStep(i, { approvers: [...s.approvers, { kind: "person", id: people.find((p) => p.kind === "member")?.id ?? "" }] })} data-testid={`step-${i}-add-approver`}>+ {s.kind === "approval" ? "approver" : "recipient"}</button>
                  </div>
                  {s.kind === "approval" && (
                    <div className="row">
                      <select value={s.mode} onChange={(e) => setStep(i, { mode: e.target.value })} data-testid={`step-mode-${i}`}>
                        <option value="any">any one of them decides</option>
                        <option value="all">all of them must approve</option>
                        <option value="sequential">one after another</option>
                      </select>
                      <label className="inline"><input type="checkbox" checked={s.canSendBack} onChange={(e) => setStep(i, { canSendBack: e.target.checked })} data-testid={`step-sendback-${i}`} /> may send back</label>
                    </div>
                  )}
                  <ConditionEditor when={s.when} fields={fields} onChange={(when) => setStep(i, { when })} testId={`step-when-${i}`} />
                </li>
              ))}
            </ol>
            <button className="secondary small" onClick={() => setSteps([...steps, { uid: uid(), kind: "approval", name: "", approvers: [], mode: "any", canSendBack: true, when: null }])} data-testid="add-step">+ Step</button>
            <p className="muted small">The requester is never asked to approve their own request. “Whoever holds a capability” asks everyone the access rules allow for that very record (limits included).</p>
          </>
        )}
      </div>
    </div>
  );
}
