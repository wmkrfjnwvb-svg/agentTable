import { useEffect, useState } from "react";
import { api, ApiError, Explanation, Person, TableSummary } from "./api";
import { ExplainList } from "./TablePage";

export const PLACEHOLDERS = [
  {
    key: "approvals",
    title: "Approvals & lifecycles",
    wave: 2,
    crate: "mt-lifecycle",
    what: [
      "States with categories (open, in progress, done …), allowed transitions and required fields per state",
      "Approval flows: sequential, all, any, quorum; delegate, return, add approvers",
      "SLA clocks on business calendars, with pauses and escalation",
    ],
    today: "The “Approve…” button on a purchase request already runs the real access check (approval limits and “nobody approves their own request”).",
  },
  {
    key: "automations",
    title: "Automations",
    wave: 5,
    crate: "mt-rules · mt-workflow",
    what: ["Rules: when a record changes, if a condition holds, do something", "Workflows with steps, waits and branches", "Scheduled jobs"],
    today: "Formulas already recompute across tables after every change.",
  },
  {
    key: "notifications",
    title: "Notifications",
    wave: 5,
    crate: "mt-notify",
    what: ["Email, chat and in-app messages from rules and workflows", "Per-person preferences and digests", "Delivery tracking"],
    today: "—",
  },
  {
    key: "dashboards",
    title: "Dashboards",
    wave: 6,
    crate: "mt-metrics",
    what: ["Certified metrics with owners", "Charts that follow the viewer's permissions", "Scheduled reports"],
    today: "Grouping with subtotals in the grid uses the same aggregation engine dashboards will use.",
  },
  {
    key: "assistant",
    title: "AI assistant",
    wave: 6,
    crate: "mt-ai · mt-mcp",
    what: [
      "AI fields (classify, extract, summarise) with provenance and review",
      "Agents that act for a person — never with more access than that person",
      "An assistant that answers questions over your tables, through MCP",
    ],
    today: "Sign in as “Purchasing assistant” acting for Bob: it sees exactly what Bob sees. That rule is already enforced.",
  },
];

export function PlaceholderPage({ which }: { which: string }) {
  const p = PLACEHOLDERS.find((x) => x.key === which);
  if (!p) return <div className="page">Unknown page.</div>;
  return (
    <div className="page" data-testid="placeholder">
      <div className="page-head">
        <div>
          <h1>{p.title}</h1>
          <p className="muted">Planned for wave {p.wave} · {p.crate}</p>
        </div>
        <span className="badge soon big">coming</span>
      </div>
      <div className="card">
        <h3>What it will do</h3>
        <ul>{p.what.map((w) => <li key={w}>{w}</li>)}</ul>
      </div>
      <div className="card">
        <h3>Already in this build</h3>
        <p>{p.today}</p>
      </div>
    </div>
  );
}

const STATE_LABEL: Record<string, string> = { live: "in this build", engine: "engine ready", placeholder: "placeholder" };

export function OverviewPage({ tables }: { tables: TableSummary[] }) {
  const [status, setStatus] = useState<Awaited<ReturnType<typeof api.status>> | null>(null);
  useEffect(() => {
    api.status().then(setStatus);
  }, []);
  return (
    <div className="page" data-testid="overview">
      <div className="page-head">
        <div>
          <h1>Product status</h1>
          <p className="muted">
            agenttable {status?.version} — an end-to-end build of everything finished so far. Unfinished parts are marked as placeholders and fill in wave by wave.
          </p>
        </div>
      </div>
      <div className="cards">
        {tables.map((t) => (
          <a className="card link" key={t.id} href={`#/table/${t.id}`}>
            <div className="card-title">{t.canRead ? "▦" : "🔒"} {t.name}</div>
            <div className="muted small">{t.description}</div>
            <div className="big-number">{t.canRead ? t.records : "—"}</div>
            <div className="muted small">{t.canRead ? "records you can see" : "no access"}</div>
          </a>
        ))}
      </div>
      <div className="card">
        <h3>Try this</h3>
        <ol className="tries">
          <li>Open <b>Request lines</b>, change a quantity — the request's subtotal, tax, total and “needs CFO” recompute, and so does the supplier's open spend.</li>
          <li>Switch to <b>Bob Buyer</b>: he sees only his own requests; bank details disappear.</li>
          <li>Switch to <b>Gina Guest</b>: she sees only her own company.</li>
          <li>As <b>Purchasing assistant</b> acting for Bob: the AI sees exactly what Bob sees.</li>
          <li>Open a request and press <b>Approve…</b> as Carol: limits and “nobody approves their own request” are explained.</li>
          <li>Add a <b>Formula</b> field and type a broken formula: the error points to the character.</li>
          <li>Open the <b>Audit log</b> as Alice or Dave and verify the hash chain; simulate tampering.</li>
        </ol>
      </div>
      <div className="card">
        <h3>Components</h3>
        <table className="status-table" data-testid="components">
          <thead>
            <tr><th>Component</th><th>Wave</th><th>State</th><th>What it does</th></tr>
          </thead>
          <tbody>
            {status?.components.map((c) => (
              <tr key={c.crate}>
                <td><code>{c.crate}</code></td>
                <td>{c.wave}</td>
                <td><span className={`badge ${c.state}`}>{STATE_LABEL[c.state] ?? c.state}</span></td>
                <td>{c.summary}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export function RefDataPage() {
  const [sets, setSets] = useState<Awaited<ReturnType<typeof api.refsets>>>([]);
  const [chosen, setChosen] = useState("");
  const [query, setQuery] = useState("");
  const [set, setSet] = useState<Awaited<ReturnType<typeof api.refset>> | null>(null);
  useEffect(() => {
    api.refsets().then((list) => {
      setSets(list);
      if (list[0]) setChosen(list[0].id);
    });
  }, []);
  useEffect(() => {
    if (!chosen) return;
    const timer = window.setTimeout(() => api.refset(chosen, query).then(setSet), query ? 150 : 0);
    return () => window.clearTimeout(timer);
  }, [chosen, query]);
  return (
    <div className="page" data-testid="refdata">
      <div className="page-head">
        <div>
          <h1>Reference data</h1>
          <p className="muted">Governed code lists that fields draw their values from. Changes with four-eyes approval are in the engine (mt-refdata); their screens come next.</p>
        </div>
      </div>
      <div className="split">
        <div className="set-list">
          {sets.map((s) => (
            <button key={s.id} className={`set ${chosen === s.id ? "chosen" : ""}`} onClick={() => { setChosen(s.id); setQuery(""); }} data-testid={`set-${s.code}`}>
              <div><b>{s.name}</b></div>
              <div className="muted small">{s.system ? "built in" : "this organisation"} · {s.tree ? "tree" : "list"} · {s.items} items</div>
            </button>
          ))}
        </div>
        <div className="set-detail card">
          {set && (
            <>
              <h3>{set.name}</h3>
              {set.description && <p className="muted">{set.description}</p>}
              <input className="search" placeholder="Search items…" value={query} onChange={(e) => setQuery(e.target.value)} data-testid="refdata-search" />
              <table className="tree" data-testid="refdata-items">
                <thead><tr><th>Item</th><th>Code</th><th>Path</th></tr></thead>
                <tbody>
                  {set.items.map((item) => (
                    <tr key={item.id}>
                      <td style={{ paddingLeft: `${0.5 + (query ? 0 : (item.depth - 1) * 1.25)}rem` }}>{set.tree && !item.leaf ? "▾ " : "• "}{item.name}</td>
                      <td><code>{item.code}</code></td>
                      <td className="muted small">{item.path}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

export function AccessPage({ people, tables }: { people: Person[]; tables: TableSummary[] }) {
  const [who, setWho] = useState(people.find((p) => p.name.startsWith("Carol"))?.id ?? "");
  const [forWhom, setForWhom] = useState("");
  const [table, setTable] = useState(tables[1]?.id ?? tables[0]?.id ?? "");
  const [records, setRecords] = useState<{ id: string; title: string }[]>([]);
  const [record, setRecord] = useState("");
  const [capability, setCapability] = useState("request.approve");
  const [catalogue, setCatalogue] = useState<Awaited<ReturnType<typeof api.capabilities>> | null>(null);
  const [answer, setAnswer] = useState<Explanation | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.capabilities().then(setCatalogue);
  }, []);
  useEffect(() => {
    if (!who && people.length) setWho(people.find((p) => p.name.startsWith("Carol"))?.id ?? people[0].id);
    if (!table && tables.length) setTable(tables[1]?.id ?? tables[0].id);
  }, [people, tables, who, table]);
  useEffect(() => {
    if (!table) return;
    // Records are listed as the administrator: the question is about someone else's access.
    const saved = { user: "", onBehalfOf: "" };
    void saved;
    fetch(`/api/tables/${table}/query`, { method: "POST", headers: { "content-type": "application/json", "x-user": "prn_00000000000000000000000001" }, body: "{}" })
      .then((r) => r.json())
      .then((r) => setRecords((r.rows ?? []).map((x: { id: string; title: string }) => ({ id: x.id, title: x.title }))));
    setRecord("");
  }, [table]);

  const ask = () => {
    setError(null);
    api
      .explain({ table, record: record || undefined, capability, as: who, for: forWhom || undefined })
      .then(setAnswer)
      .catch((e: ApiError) => setError(e.message));
  };
  const person = people.find((p) => p.id === who);

  return (
    <div className="page" data-testid="access">
      <div className="page-head">
        <div>
          <h1>Access &amp; explain</h1>
          <p className="muted">Ask whether someone may do something — and why. The same engine decides every screen, export and AI call.</p>
        </div>
      </div>
      <div className="card question">
        <label>
          Who
          <select value={who} onChange={(e) => setWho(e.target.value)} data-testid="explain-who">
            {people.filter((p) => p.kind !== "group").map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
          </select>
        </label>
        {person?.kind === "agent" && (
          <label>
            acting for
            <select value={forWhom} onChange={(e) => setForWhom(e.target.value)}>
              <option value="">nobody</option>
              {people.filter((p) => p.kind === "member" || p.kind === "guest").map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
            </select>
          </label>
        )}
        <label>
          may
          <select value={capability} onChange={(e) => setCapability(e.target.value)} data-testid="explain-capability">
            {catalogue?.capabilities.map((c) => <option key={c} value={c}>{c}</option>)}
          </select>
        </label>
        <label>
          in
          <select value={table} onChange={(e) => setTable(e.target.value)} data-testid="explain-table">
            {tables.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
          </select>
        </label>
        <label>
          record
          <select value={record} onChange={(e) => setRecord(e.target.value)} data-testid="explain-record">
            <option value="">(the table as a whole)</option>
            {records.map((r) => <option key={r.id} value={r.id}>{r.title}</option>)}
          </select>
        </label>
        <button onClick={ask} data-testid="explain-ask">Explain</button>
      </div>
      {error && <div className="notice refused">{error}</div>}
      {answer && (
        <div className={`card answer ${answer.allowed ? "allowed" : "refused"}`} data-testid="explain-answer">
          <h2>{answer.allowed ? "✓ Allowed" : "✕ Not allowed"}</h2>
          <p className="muted">
            {answer.question?.who}{answer.question?.for ? ` (acting for ${answer.question.for})` : ""} · <code>{answer.question?.capability}</code> · {answer.question?.table}
            {answer.question?.record ? ` · “${answer.question.record}”` : ""}
          </p>
          {answer.chain.length > 0 && (
            <>
              <h3>Because of</h3>
              <ul className="chain">
                {answer.chain.map((c, i) => (
                  <li key={i}>
                    <b>{c.principal}</b> {c.viaGroup ? <>is in <b>{c.holder}</b>, which</> : ""} holds <code>{c.capability}</code>
                    {c.set ? <> from the set “{c.set}”</> : null} on {c.on}
                    {c.scope ? <> — {c.scope}</> : null}. <span className="muted">“{c.reason}”, given by {c.grantedBy}.</span>
                  </li>
                ))}
              </ul>
            </>
          )}
          {answer.refusals.length > 0 && (
            <>
              <h3>{answer.allowed ? "Grants that did not apply" : "Why not"}</h3>
              <ExplainList details={answer} />
            </>
          )}
          {answer.allowed && answer.fields && <p className="small muted">Fields covered: {answer.fields.join(", ")}</p>}
        </div>
      )}
      {catalogue && catalogue.rules.length > 0 && (
        <div className="card">
          <h3>Separation-of-duties rules</h3>
          <ul>
            {catalogue.rules.map((r) => (
              <li key={r.name}>
                <b>{r.name}</b>: <code>{r.first}</code> and <code>{r.second}</code> {r.kind === "hold" ? "are never held by the same person" : "are never used by the same person on one record"}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

export function AuditPage() {
  const [entries, setEntries] = useState<Awaited<ReturnType<typeof api.audit>>>([]);
  const [error, setError] = useState<ApiError | null>(null);
  const [verdict, setVerdict] = useState<Awaited<ReturnType<typeof api.verify>> | null>(null);
  const [tamper, setTamper] = useState<Awaited<ReturnType<typeof api.tamper>> | null>(null);
  useEffect(() => {
    api.audit().then(setEntries).catch((e: ApiError) => setError(e));
  }, []);
  if (error) {
    return (
      <div className="page">
        <h1>Audit log</h1>
        <div className="notice refused" data-testid="no-access">
          {error.message}
          <ExplainList details={error.details} />
        </div>
      </div>
    );
  }
  const middle = entries.length ? entries[Math.floor(entries.length / 2)].seq : 1;
  return (
    <div className="page" data-testid="audit">
      <div className="page-head">
        <div>
          <h1>Audit log</h1>
          <p className="muted">Every change is an entry in a hash chain: each entry seals the one before it, so any later edit is detectable.</p>
        </div>
        <div className="actions">
          <button onClick={() => api.verify().then(setVerdict)} data-testid="verify">Verify chain</button>
          <button className="secondary" onClick={() => api.tamper(middle).then(setTamper)} data-testid="tamper">Simulate tampering with #{middle}</button>
        </div>
      </div>
      {verdict && (
        <div className={`notice ${verdict.intact ? "ok" : "refused"}`} data-testid="verdict">
          {verdict.intact ? `✓ Chain intact — ${verdict.entries} entries verified. Head ${verdict.head?.slice(0, 16)}…` : `✕ Chain damaged at entry ${verdict.seq}`}
        </div>
      )}
      {tamper && (
        <div className="notice refused" data-testid="tamper-result">
          On a copy with entry #{tamper.seq} altered, verification reports: <b>{tamper.kind}</b> at entry {tamper.seq}; the last trustworthy entry is #{tamper.lastGood}. (The real log was not changed.)
        </div>
      )}
      <div className="grid-wrap">
        <table className="grid audit-table" data-testid="audit-entries">
          <thead>
            <tr><th>#</th><th>When (UTC)</th><th>Who</th><th>Action</th><th>What</th><th>Hash</th></tr>
          </thead>
          <tbody>
            {entries.map((e) => {
              const details = (e.details ?? {}) as { title?: string; changes?: { field: string; from: string; to: string }[]; field?: string; type?: string };
              return (
                <tr key={e.seq}>
                  <td>{e.seq}</td>
                  <td className="small">{e.at.replace("T", " ").slice(0, 19)}</td>
                  <td>{e.actor}{e.initiator !== e.actor ? <span className="muted"> for {e.initiator}</span> : null}</td>
                  <td><code>{e.action}</code></td>
                  <td>
                    {details.title ?? details.field ?? e.object}
                    {details.type && <span className="muted"> ({details.type})</span>}
                    {details.changes?.map((c, i) => (
                      <div key={i} className="small muted">{c.field}: {c.from || "—"} → {c.to || "—"}</div>
                    ))}
                  </td>
                  <td className="hash" title={e.hash}>{e.hash.slice(0, 10)}…</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
