import { useEffect, useState } from "react";
import { api, ApiError, BaseSummary, Person } from "./api";
import { ConditionEditor, FieldInfo, When } from "./Designer";

// Building access: grants (who may do what, where, to which records, up to which limit, and
// why), capability sets and separation-of-duties rules. mt-perm decides what is allowed; it
// refuses grants that break a rule, and keeps revoked grants on record.

type Overview = Awaited<ReturnType<typeof api.access>>;

export default function AccessBuilder({ people }: { people: Person[] }) {
  const [overview, setOverview] = useState<Overview | null>(null);
  const [bases, setBases] = useState<BaseSummary[]>([]);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  // The grant form.
  const [to, setTo] = useState("");
  const [kind, setKind] = useState<"capability" | "set">("capability");
  const [capability, setCapability] = useState("record.read");
  const [set, setSet] = useState("");
  const [place, setPlace] = useState("workspace");
  const [scope, setScope] = useState("all");
  const [limit, setLimit] = useState<When | null>(null);
  const [fields, setFields] = useState<FieldInfo[]>([]);
  const [reason, setReason] = useState("");
  // The set and rule forms.
  const [setName, setSetName] = useState("");
  const [setCaps, setSetCaps] = useState<string[]>([]);
  const [ruleName, setRuleName] = useState("");
  const [first, setFirst] = useState("");
  const [second, setSecond] = useState("");
  const [ruleKind, setRuleKind] = useState("hold");

  const load = () => {
    api.access().then(setOverview).catch((e: ApiError) => setMessage({ ok: false, text: e.message }));
    api.bases().then((r) => setBases(r.bases)).catch(() => setBases([]));
  };
  useEffect(load, []);
  useEffect(() => {
    if (!to && people.length) setTo(people.find((p) => p.kind !== "agent")?.id ?? "");
  }, [people, to]);
  useEffect(() => {
    setLimit(null);
    if (!place.startsWith("table:")) {
      setFields([]);
      return;
    }
    api
      .table(place.slice(6))
      .then((t) => setFields(t.columns.map((c) => ({ id: c.id, name: c.name, dataType: c.dataType, type: c.type, generated: c.generated, options: c.options?.filter((o) => o.enabled).map((o) => ({ id: o.id, label: o.label })) ?? null }))))
      .catch(() => setFields([]));
  }, [place]);

  const attempt = async (work: () => Promise<unknown>, done: string) => {
    setMessage(null);
    try {
      await work();
      setMessage({ ok: true, text: done });
      load();
    } catch (e) {
      setMessage({ ok: false, text: (e as ApiError).message });
    }
  };
  const on = place === "workspace" ? { kind: "workspace" } : place.startsWith("base:") ? { kind: "base", id: place.slice(5) } : { kind: "table", id: place.slice(6) };
  const give = () =>
    attempt(
      () => api.giveGrant({ to, capability: kind === "capability" ? capability : undefined, set: kind === "set" ? set : undefined, on, scope, limit: limit ?? undefined, reason }),
      "Given.",
    );

  if (!overview) return null;
  const grantees = people.filter((p) => p.kind !== "app");
  return (
    <div data-testid="access-builder">
      {message && <div className={`notice ${message.ok ? "ok" : "refused"}`} data-testid="access-message">{message.text}</div>}
      <div className="card">
        <h3>Who may do what</h3>
        <table className="plain" data-testid="grants">
          <thead><tr><th>Who</th><th>May</th><th>Where</th><th>Which records</th><th>Limit</th><th>Why</th><th /></tr></thead>
          <tbody>
            {overview.grants.map((g) => (
              <tr key={g.id} className={g.active ? "" : "inactive"} data-testid="grant-row">
                <td>{g.to}</td>
                <td><code>{g.what}</code></td>
                <td>{g.on}</td>
                <td>{g.scope}</td>
                <td>{g.limit ?? ""}</td>
                <td className="muted small">{g.reason} · {g.grantedBy}</td>
                <td>
                  {g.active ? (
                    overview.canManage && <button className="ghost small" onClick={() => attempt(() => api.revokeGrant(g.id), "Revoked.")} data-testid={`revoke-${g.to}-${g.what}`}>Revoke</button>
                  ) : (
                    <span className="muted small">revoked</span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {overview.canManage && (
          <div className="grant-form">
            <h3>Give access</h3>
            <div className="form-row">
              <select value={to} onChange={(e) => setTo(e.target.value)} data-testid="grant-to">
                {grantees.map((p) => <option key={p.id} value={p.id}>{p.name}{p.kind === "group" ? " (group)" : ""}</option>)}
              </select>
              <span>may</span>
              <select value={kind} onChange={(e) => setKind(e.target.value as "capability" | "set")} data-testid="grant-kind">
                <option value="capability">do</option>
                <option value="set">everything in the set</option>
              </select>
              {kind === "capability" ? (
                <>
                  <input list="capability-list" value={capability} onChange={(e) => setCapability(e.target.value)} data-testid="grant-capability" />
                  <datalist id="capability-list">{overview.capabilities.map((c) => <option key={c} value={c} />)}</datalist>
                </>
              ) : (
                <select value={set} onChange={(e) => setSet(e.target.value)} data-testid="grant-set">
                  <option value="">…</option>
                  {overview.sets.map((s) => <option key={s.id} value={s.id}>{s.name}</option>)}
                </select>
              )}
            </div>
            <div className="form-row">
              <span>in</span>
              <select value={place} onChange={(e) => setPlace(e.target.value)} data-testid="grant-place">
                <option value="workspace">the whole workspace</option>
                {bases.map((b) => [
                  <option key={b.id} value={`base:${b.id}`}>the base {b.name}</option>,
                  ...b.tables.map((t) => <option key={t.id} value={`table:${t.id}`}>  the table {t.name}</option>),
                ])}
              </select>
              <span>for</span>
              <select value={scope} onChange={(e) => setScope(e.target.value)} data-testid="grant-scope">
                <option value="all">all records</option>
                <option value="own">records they own (created, requested, assigned)</option>
                <option value="team">records of their team</option>
              </select>
              {place.startsWith("table:") && <ConditionEditor when={limit} fields={fields} onChange={setLimit} testId="grant-limit" />}
            </div>
            <div className="form-row">
              <input value={reason} onChange={(e) => setReason(e.target.value)} placeholder="Why (shown when access is explained)" data-testid="grant-reason" />
              <button onClick={give} disabled={!to || (kind === "set" ? !set : !capability.trim())} data-testid="give-grant">Give</button>
            </div>
          </div>
        )}
      </div>
      <div className="two-columns">
        <div className="card">
          <h3>Capability sets</h3>
          <ul>
            {overview.sets.map((s) => (
              <li key={s.id}><b>{s.name}</b>: <span className="muted small">{s.capabilities.join(", ")}</span></li>
            ))}
          </ul>
          {overview.canManage && (
            <>
              <input value={setName} onChange={(e) => setSetName(e.target.value)} placeholder="Set name, e.g. Employee" data-testid="set-name" />
              <div className="chips wrap">
                {overview.capabilities.map((c) => (
                  <label key={c} className={`chip-toggle ${setCaps.includes(c) ? "on" : ""}`} data-testid={`set-cap-${c}`}>
                    <input type="checkbox" checked={setCaps.includes(c)} onChange={() => setSetCaps(setCaps.includes(c) ? setCaps.filter((x) => x !== c) : [...setCaps, c])} />
                    {c}
                  </label>
                ))}
              </div>
              <button
                className="secondary"
                onClick={() => attempt(async () => { await api.createSet(setName, setCaps); setSetName(""); setSetCaps([]); }, "Set created.")}
                disabled={!setName.trim() || setCaps.length === 0}
                data-testid="create-cap-set"
              >
                Create set
              </button>
            </>
          )}
        </div>
        <div className="card">
          <h3>Separation of duties</h3>
          <ul>
            {overview.rules.map((r) => (
              <li key={r.name}><b>{r.name}</b>: <code>{r.first}</code> and <code>{r.second}</code> {r.kind === "hold" ? "are never held by the same person" : "are never used by the same person on one record"}</li>
            ))}
          </ul>
          {overview.canManage && (
            <div className="form-row">
              <input value={ruleName} onChange={(e) => setRuleName(e.target.value)} placeholder="Rule name" data-testid="rule-name" />
              <input list="capability-list" value={first} onChange={(e) => setFirst(e.target.value)} placeholder="capability" data-testid="rule-first" />
              <input list="capability-list" value={second} onChange={(e) => setSecond(e.target.value)} placeholder="and capability" data-testid="rule-second" />
              <select value={ruleKind} onChange={(e) => setRuleKind(e.target.value)} data-testid="rule-kind">
                <option value="hold">never held by one person</option>
                <option value="same_record">never both on one record</option>
              </select>
              <button className="secondary" onClick={() => attempt(() => api.addRule(ruleName, first, second, ruleKind), "Rule added.")} disabled={!ruleName.trim() || !first.trim() || !second.trim()} data-testid="add-rule">Add rule</button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
