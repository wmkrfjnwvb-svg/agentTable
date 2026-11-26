import { useEffect, useState } from "react";
import { AccessReview, api, ApiError, BaseSummary, GrantRow, Person, ShareRow } from "./api";
import { ConditionEditor, FieldInfo, When } from "./Designer";

// Building access: grants (who may do what, where, to which records, up to which limit, and
// why), capability sets and separation-of-duties rules. mt-perm decides what is allowed; it
// refuses grants that break a rule, and keeps revoked grants on record.

type Overview = Awaited<ReturnType<typeof api.access>>;

/// A date input's value as the start of that day (UTC), in milliseconds.
export function dayStart(date: string): number | undefined {
  return date ? Date.parse(`${date}T00:00:00Z`) : undefined;
}

/// Milliseconds as a date.
export function day(millis: number | null | undefined): string {
  return millis == null ? "" : new Date(millis).toISOString().slice(0, 10);
}

function validity(g: GrantRow): string {
  switch (g.status) {
    case "revoked":
      return "revoked";
    case "not_yet":
      return `from ${day(g.validFrom)}`;
    case "ended":
      return `ended ${day(g.validTo)}`;
    case "in_force":
      return g.validTo ? `until ${day(g.validTo)}` : "";
  }
}

function placeSpec(place: string) {
  return place === "workspace" ? { kind: "workspace" } : place.startsWith("base:") ? { kind: "base", id: place.slice(5) } : { kind: "table", id: place.slice(6) };
}

function PlaceSelect({ value, onChange, bases, testId }: { value: string; onChange: (v: string) => void; bases: BaseSummary[]; testId: string }) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value)} data-testid={testId}>
      <option value="workspace">the whole workspace</option>
      {bases.map((b) => [
        <option key={b.id} value={`base:${b.id}`}>the base {b.name}</option>,
        ...b.tables.map((t) => <option key={t.id} value={`table:${t.id}`}>  the table {t.name}</option>),
      ])}
    </select>
  );
}

/// Choosing fields: every field, only some, or all but some.
export function FieldChooser({ fields, mode, chosen, onMode, onChosen, testId, modes = ["all", "only", "except"] }: { fields: FieldInfo[]; mode: string; chosen: string[]; onMode: (m: string) => void; onChosen: (c: string[]) => void; testId: string; modes?: string[] }) {
  const label: Record<string, string> = { all: "all fields", only: "only the fields", except: "all fields but" };
  return (
    <span className="field-chooser">
      <select value={mode} onChange={(e) => onMode(e.target.value)} data-testid={`${testId}-mode`}>
        {modes.map((m) => <option key={m} value={m}>{label[m]}</option>)}
      </select>
      {mode !== "all" && (
        <span className="chips wrap">
          {fields.map((f) => (
            <label key={f.id} className={`chip-toggle ${chosen.includes(f.id) ? "on" : ""}`} data-testid={`${testId}-${f.name}`}>
              <input type="checkbox" checked={chosen.includes(f.id)} onChange={() => onChosen(chosen.includes(f.id) ? chosen.filter((x) => x !== f.id) : [...chosen, f.id])} />
              {f.name}
            </label>
          ))}
        </span>
      )}
    </span>
  );
}

function ReviewCard({ review, canManage, attempt }: { review: AccessReview; canManage: boolean; attempt: (work: () => Promise<unknown>, done: string) => Promise<void> }) {
  const [notes, setNotes] = useState<Record<string, string>>({});
  const open = review.state === "open";
  return (
    <div className="review" data-testid={`review-${review.name}`}>
      <div className="row wrap">
        <b>{review.name}</b>
        <span className="muted small">
          {review.covers} · opened by {review.openedBy}
          {review.dueAt ? ` · due ${day(review.dueAt)}` : ""}
        </span>
        <span className={`badge ${review.state === "completed" ? "ok" : review.overdue ? "refused" : ""}`} data-testid="review-state">
          {review.state === "open" ? `${review.done} of ${review.total} decided${review.overdue ? " · overdue" : ""}` : `${review.state}${review.closedBy ? ` by ${review.closedBy}` : ""}`}
        </span>
      </div>
      <table className="plain">
        <tbody>
          {review.items.map((item) => (
            <tr key={item.grant} data-testid="review-item">
              <td>{item.holder}</td>
              <td>
                <code>{item.what?.what}</code> <span className="muted small">{item.what?.on} · {item.what?.scope}{item.what?.fields ? ` · ${item.what.fields}` : ""}</span>
              </td>
              <td>
                {item.verdict && (
                  <span className={`badge ${item.verdict === "keep" ? "ok" : "refused"}`} data-testid="verdict">
                    {item.verdict === "keep" ? "keep" : "revoke"}
                    {item.reviewer ? ` · ${item.reviewer}` : ""}
                  </span>
                )}
                {item.note && <span className="muted small"> “{item.note}”</span>}
              </td>
              <td>
                {open && canManage && (
                  <span className="row">
                    <button className="ghost small" onClick={() => attempt(() => api.decideReview(review.id, item.grant, "keep"), "Kept.")} data-testid={`keep-${item.holder}`}>Keep</button>
                    <input className="small-input" value={notes[item.grant] ?? ""} onChange={(e) => setNotes({ ...notes, [item.grant]: e.target.value })} placeholder="why revoke" data-testid={`note-${item.holder}`} />
                    <button className="ghost small danger-text" onClick={() => attempt(() => api.decideReview(review.id, item.grant, "revoke", notes[item.grant]), "Marked for revocation.")} data-testid={`revoke-verdict-${item.holder}`}>Revoke</button>
                  </span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {open && canManage && (
        <div className="row">
          <button onClick={() => attempt(() => api.completeReview(review.id), "Review completed.")} data-testid="complete-review">Complete review</button>
          <button className="secondary" onClick={() => attempt(() => api.cancelReview(review.id), "Review cancelled.")} data-testid="cancel-review">Cancel review</button>
        </div>
      )}
    </div>
  );
}

function ShareList({ shares, canManage, attempt }: { shares: ShareRow[]; canManage: boolean; attempt: (work: () => Promise<unknown>, done: string) => Promise<void> }) {
  if (shares.length === 0) return <p className="muted small">No links yet. Open a record and choose “Share”.</p>;
  return (
    <table className="plain" data-testid="shares">
      <thead><tr><th>Opens</th><th>Fields</th><th>Expires</th><th>By</th><th>Link</th><th /></tr></thead>
      <tbody>
        {shares.map((s) => (
          <tr key={s.id} className={s.status === "active" ? "" : "inactive"} data-testid="share-row">
            <td>{s.table}{s.record ? ` › ${s.record}` : " (all records)"}</td>
            <td className="small">{s.fields ?? "all"}</td>
            <td className="small">{s.expiresAt ? day(s.expiresAt) : "never"}</td>
            <td className="small">{s.createdBy}</td>
            <td className="small">
              {s.status === "active" ? (s.token ? <a href={`#/share/${s.token}`} target="_blank" rel="noreferrer">open</a> : "") : <span className="muted">{s.status}</span>}
            </td>
            <td>{s.status === "active" && canManage && <button className="ghost small" onClick={() => attempt(() => api.revokeShare(s.id), "Link withdrawn.")} data-testid={`withdraw-${s.record ?? s.table}`}>Withdraw</button>}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

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
  const [fieldMode, setFieldMode] = useState("all");
  const [chosen, setChosen] = useState<string[]>([]);
  const [from, setFrom] = useState("");
  const [until, setUntil] = useState("");
  // The review form.
  const [reviewName, setReviewName] = useState("");
  const [reviewPlace, setReviewPlace] = useState("workspace");
  const [reviewDue, setReviewDue] = useState("");
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
    setFieldMode("all");
    setChosen([]);
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
  const on = placeSpec(place);
  const give = () =>
    attempt(
      () =>
        api.giveGrant({
          to,
          capability: kind === "capability" ? capability : undefined,
          set: kind === "set" ? set : undefined,
          on,
          scope,
          limit: limit ?? undefined,
          reason,
          fields: fieldMode === "only" ? { only: chosen } : fieldMode === "except" ? { except: chosen } : undefined,
          validFrom: dayStart(from),
          validTo: dayStart(until),
        }),
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
          <thead><tr><th>Who</th><th>May</th><th>Where</th><th>Which records</th><th>Fields</th><th>Limit</th><th>When</th><th>Why</th><th /></tr></thead>
          <tbody>
            {overview.grants.map((g) => (
              <tr key={g.id} className={g.status === "in_force" ? "" : "inactive"} data-testid="grant-row">
                <td>{g.to}</td>
                <td><code>{g.what}</code></td>
                <td>{g.on}</td>
                <td>{g.scope}</td>
                <td className="small">{g.fields ?? ""}</td>
                <td>{g.limit ?? ""}</td>
                <td className="small" data-testid="grant-validity">{validity(g)}</td>
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
              <PlaceSelect value={place} onChange={setPlace} bases={bases} testId="grant-place" />
              <span>for</span>
              <select value={scope} onChange={(e) => setScope(e.target.value)} data-testid="grant-scope">
                <option value="all">all records</option>
                <option value="own">records they own (created, requested, assigned)</option>
                <option value="team">records of their team</option>
              </select>
              {place.startsWith("table:") && <ConditionEditor when={limit} fields={fields} onChange={setLimit} testId="grant-limit" />}
            </div>
            <div className="form-row">
              {place.startsWith("table:") && (
                <>
                  <span>seeing</span>
                  <FieldChooser fields={fields} mode={fieldMode} chosen={chosen} onMode={setFieldMode} onChosen={setChosen} testId="grant-fields" />
                </>
              )}
              <label className="inline small">from <input type="date" value={from} onChange={(e) => setFrom(e.target.value)} data-testid="grant-from" /></label>
              <label className="inline small">until <input type="date" value={until} onChange={(e) => setUntil(e.target.value)} data-testid="grant-until" /></label>
            </div>
            <div className="form-row">
              <input value={reason} onChange={(e) => setReason(e.target.value)} placeholder="Why (shown when access is explained)" data-testid="grant-reason" />
              <button onClick={give} disabled={!to || (kind === "set" ? !set : !capability.trim())} data-testid="give-grant">Give</button>
            </div>
          </div>
        )}
      </div>
      <div className="card" data-testid="reviews">
        <h3>Access reviews</h3>
        <p className="muted small">Someone other than the holder confirms that each grant is still needed. Grants marked “revoke” are revoked when the review is completed, with the note in the audit trail.</p>
        {overview.reviews.map((r) => <ReviewCard key={r.id} review={r} canManage={overview.canManage} attempt={attempt} />)}
        {overview.canManage && (
          <div className="form-row">
            <input value={reviewName} onChange={(e) => setReviewName(e.target.value)} placeholder="Review name, e.g. Q4 expenses" data-testid="review-name" />
            <span>of the grants in</span>
            <PlaceSelect value={reviewPlace} onChange={setReviewPlace} bases={bases} testId="review-place" />
            <label className="inline small">due <input type="date" value={reviewDue} onChange={(e) => setReviewDue(e.target.value)} data-testid="review-due" /></label>
            <button className="secondary" onClick={() => attempt(async () => { await api.openReview(reviewName, placeSpec(reviewPlace), dayStart(reviewDue)); setReviewName(""); }, "Review opened.")} disabled={!reviewName.trim()} data-testid="open-review">Open review</button>
          </div>
        )}
      </div>
      <div className="card">
        <h3>Share links</h3>
        <p className="muted small">Read-only links for people without an account. A link never shows more than the person who made it may see, never shows confidential or protected fields, and stops working when it expires, is withdrawn, or its maker loses the right to share.</p>
        <ShareList shares={overview.shares} canManage={overview.canManage} attempt={attempt} />
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
