import { useEffect, useState } from "react";
import { Act, api, ApiError, ApprovalRun, MyTask, Notice, ProcessDefinitions, ProcessInfo, RecordDetail, RunSummary, StepNode } from "./api";

// Lifecycles and approvals: the state and buttons of a record, its approval timeline, the
// approver's decision box, "My approvals", the process design and the inbox. Every rule
// (who may, who is asked, what is locked) is decided by the server (mt-lifecycle + mt-perm).

/// An instant as "2026-11-18 10:00" (UTC). The server sends milliseconds since the epoch.
export function when(at: string | number | null | undefined): string {
  if (at === null || at === undefined || at === "") return "";
  const date = new Date(typeof at === "number" ? at : Number.isNaN(Number(at)) ? at : Number(at));
  if (Number.isNaN(date.getTime())) return String(at);
  return date.toISOString().slice(0, 16).replace("T", " ");
}

function StateBadge({ name, category }: { name: string; category: string | null }) {
  return (
    <span className={`state state-${category ?? "new"}`} data-testid="state">
      {name}
    </span>
  );
}

/// The states of the lifecycle with the current one highlighted (terminal alternatives shown
/// only when reached).
function Stepper({ process }: { process: ProcessInfo }) {
  const current = process.state.key;
  const shown = process.states.filter(
    (s) => s.key === current || !["done_failure", "cancelled"].includes(s.category),
  );
  const index = shown.findIndex((s) => s.key === current);
  return (
    <ol className="stepper">
      {shown.map((s, i) => (
        <li key={s.key} className={i < index ? "past" : i === index ? `now state-${s.category}` : "future"}>
          {s.name}
        </li>
      ))}
    </ol>
  );
}

export function Timeline({ run }: { run: ApprovalRun }) {
  return (
    <div className="timeline" data-testid="timeline">
      <div className="timeline-head">
        <b>{run.definition ?? "Approval"}</b> <span className="muted small">v{run.version}</span>
        <span className={`run-status run-${run.status.replace(" ", "-")}`} data-testid="approval-status">
          {run.status}
        </span>
      </div>
      <ul className="tasks">
        {run.tasks.map((t) => (
          <li key={t.id} className={`task task-${t.status.replace(/ /g, "-")}`} data-testid="timeline-task">
            <span className="task-dot" aria-hidden />
            <div>
              <div>
                <b>{t.step}</b> · {t.assignee}
                {t.delegatedFrom && <span className="muted small"> (from {t.delegatedFrom})</span>}
                <span className="task-status">{t.status}</span>
              </div>
              {t.comment && <div className="small comment">“{t.comment}”</div>}
              <div className="muted small">{t.actedAt ? `decided ${when(t.actedAt)}` : `asked ${when(t.openedAt)}`}</div>
            </div>
          </li>
        ))}
      </ul>
      <details className="history">
        <summary className="small muted">History ({run.history.length})</summary>
        <ul>
          {run.history.map((h, i) => (
            <li key={i} className="small">
              <span className="muted">{when(h.at)}</span> {h.actor}: {h.what}
            </li>
          ))}
        </ul>
      </details>
    </div>
  );
}

/// The approver's box: comment, approve, reject, send back.
export function DecisionBox({ run, task, canSendBack, onDone }: { run: number; task: number; canSendBack: boolean; onDone: (r: RecordDetail) => void }) {
  const [comment, setComment] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const send = async (act: Act) => {
    setBusy(true);
    setMessage(null);
    try {
      const record = await api.act(run, task, act);
      setComment("");
      onDone(record);
    } catch (e) {
      setMessage((e as ApiError).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="decision-box" data-testid="decision-box">
      <textarea
        rows={2}
        placeholder="Comment (needed to reject or send back)"
        value={comment}
        onChange={(e) => setComment(e.target.value)}
        data-testid="task-comment"
      />
      <div className="row">
        <button onClick={() => send({ act: "approve", comment: comment || undefined })} disabled={busy} data-testid="task-approve">
          Approve
        </button>
        <button className="danger" onClick={() => send({ act: "reject", comment })} disabled={busy} data-testid="task-reject">
          Reject
        </button>
        {canSendBack && (
          <button className="secondary" onClick={() => send({ act: "return", comment })} disabled={busy} data-testid="task-return">
            Send back
          </button>
        )}
      </div>
      {message && <div className="notice refused small" data-testid="task-message">{message}</div>}
    </div>
  );
}

/// The process part of the record panel.
export function ProcessSection({ tableId, record, onChanged }: { tableId: string; record: RecordDetail; onChanged: (r: RecordDetail) => void }) {
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const process = record.process;
  if (!process) return null;
  const take = async (key: string) => {
    setBusy(true);
    setMessage(null);
    try {
      onChanged(await api.transition(tableId, record.id, key));
    } catch (e) {
      setMessage((e as ApiError).message);
    } finally {
      setBusy(false);
    }
  };
  const mine = process.approval?.tasks.find((t) => t.mine);
  return (
    <div className="process" data-testid="process">
      <div className="process-head">
        <StateBadge name={process.state.name} category={process.state.category} />
        <Stepper process={process} />
      </div>
      {process.transitions.length > 0 && (
        <div className="transitions">
          {process.transitions.map((t) => (
            <span key={t.key} className="transition">
              <button
                className={t.startsApproval ? "" : "secondary"}
                disabled={!t.allowed || busy}
                onClick={() => take(t.key)}
                title={t.reasons.join("\n")}
                data-testid={`transition-${t.key}`}
              >
                {t.name}
              </button>
              {!t.allowed && t.reasons.length > 0 && (
                <span className="muted small reason" data-testid={`why-${t.key}`}>
                  {t.reasons.join(" · ")}
                </span>
              )}
            </span>
          ))}
        </div>
      )}
      {message && <div className="notice refused" data-testid="process-message">{message}</div>}
      {process.wouldAsk && process.state.category === "new" && (
        <div className="would-ask small" data-testid="would-ask">
          {process.wouldAsk.outcome === "stuck" ? (
            <span className="bad">Nobody may approve this amount: submitting would get stuck.</span>
          ) : (
            <>
              Submitting asks{" "}
              {process.wouldAsk.steps.map((s, i) => (
                <span key={i}>
                  {i > 0 && ", then "}
                  <b>{s.people.join(" or ")}</b> <span className="muted">({s.step})</span>
                </span>
              ))}
              .
            </>
          )}
        </div>
      )}
      {mine && process.approval && (
        <div className="your-turn">
          <div className="small"><b>Your decision</b> · {mine.step}</div>
          <DecisionBox run={process.approval.id} task={mine.id} canSendBack={mine.canSendBack} onDone={onChanged} />
        </div>
      )}
      {process.approval && <Timeline run={process.approval} />}
    </div>
  );
}

// ------------------------------------------------------------------- pages

function Steps({ nodes }: { nodes: StepNode[] }) {
  return (
    <ol className="flow">
      {nodes.map((n) => (
        <li key={n.key} className={`flow-${n.kind}`}>
          <div className="flow-node">
            <span className="flow-kind">{n.kind.replace("_", " ")}</span> <b>{n.name}</b>
            {n.who && n.who.length > 0 && (
              <div className="small">
                {n.kind === "notify" ? "tell " : ""}
                {n.who.join(", ")}
                {n.mode ? ` — ${n.mode}` : ""}
                {n.canSendBack ? " · may send back" : ""}
              </div>
            )}
            {n.fields && <div className="small">fields: {n.fields.join(", ")}</div>}
          </div>
          {n.cases && (
            <div className="flow-cases">
              {n.cases.map((c) => (
                <div key={c.name} className="flow-case">
                  <div className="small muted">if {c.name}</div>
                  {c.then.length ? <Steps nodes={c.then} /> : <div className="small muted">nothing more</div>}
                </div>
              ))}
              <div className="flow-case">
                <div className="small muted">otherwise</div>
                {n.otherwise && n.otherwise.length ? <Steps nodes={n.otherwise} /> : <div className="small muted">nothing more</div>}
              </div>
            </div>
          )}
          {n.branches && (
            <div className="flow-cases">
              {n.branches.map((b, i) => (
                <div key={i} className="flow-case">
                  <Steps nodes={b} />
                </div>
              ))}
              <div className="small muted">continues after {n.join}</div>
            </div>
          )}
        </li>
      ))}
    </ol>
  );
}

function ProcessDesign() {
  const [defs, setDefs] = useState<ProcessDefinitions | null>(null);
  useEffect(() => {
    api.process().then(setDefs).catch(() => setDefs(null));
  }, []);
  if (!defs) return <div className="muted">Loading…</div>;
  return (
    <div className="design" data-testid="process-design">
      {defs.lifecycles.map((l) => (
        <div className="card" key={l.name}>
          <h3>
            Lifecycle: {l.name} <span className="muted small">on {l.table}</span>
          </h3>
          <div className="states">
            {l.states.map((s) => (
              <div key={s.key} className={`state-card state-${s.category}`}>
                <b>{s.name}</b>
                <div className="small muted">{s.category.replace("_", " ")}</div>
                {s.required.length > 0 && <div className="small">needs {s.required.join(", ")}</div>}
                {s.frozen && <div className="small">fields locked</div>}
              </div>
            ))}
          </div>
          <table className="plain">
            <thead>
              <tr><th>Button</th><th>From</th><th>To</th><th>Who</th><th>Starts</th></tr>
            </thead>
            <tbody>
              {l.transitions.map((t) => (
                <tr key={t.key}>
                  <td>{t.name}</td>
                  <td>{t.from.join(", ")}</td>
                  <td>{t.to}</td>
                  <td>{t.who}</td>
                  <td>{t.approval ?? ""}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ))}
      {defs.approvals.map((a) => (
        <div className="card" key={a.key}>
          <h3>
            Approval: {a.name} <span className="muted small">version {a.version}</span>
          </h3>
          <p className="small muted">
            Approvers named by a capability are whoever the access rules allow for that very record — limits and
            “nobody approves their own request” included. The requester is never asked.
          </p>
          <Steps nodes={a.steps} />
        </div>
      ))}
    </div>
  );
}

function TaskList({ onCount }: { onCount: (n: number) => void }) {
  const [tasks, setTasks] = useState<MyTask[] | null>(null);
  const [selected, setSelected] = useState<MyTask | null>(null);
  const [record, setRecord] = useState<RecordDetail | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const load = () =>
    api
      .tasks()
      .then((t) => {
        setTasks(t);
        onCount(t.length);
      })
      .catch(() => setTasks([]));
  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    if (selected) api.record(selected.table, selected.record).then(setRecord).catch(() => setRecord(null));
    else setRecord(null);
  }, [selected]);
  if (!tasks) return <div className="muted">Loading…</div>;
  return (
    <div className="split">
      <div className="task-list" data-testid="my-tasks">
        {tasks.length === 0 && <div className="empty muted" data-testid="no-tasks">Nothing waits for you.</div>}
        {tasks.map((t) => (
          <button
            key={`${t.run}-${t.task}`}
            className={`task-card ${selected?.run === t.run && selected.task === t.task ? "selected" : ""}`}
            onClick={() => setSelected(t)}
            data-testid="task-card"
          >
            <div className="task-title">{t.title}</div>
            <div className="small muted">
              {t.step} · from {t.requester}
              {t.delegatedFrom ? ` · handed over by ${t.delegatedFrom}` : ""}
            </div>
            {t.total && <div className="task-total">{t.total}</div>}
          </button>
        ))}
        {done && <div className="notice ok" data-testid="task-done">{done}</div>}
      </div>
      <div className="task-detail">
        {selected && record?.process?.approval ? (
          <div className="card">
            <h3 data-testid="task-record">{record.title}</h3>
            <div className="small muted">
              {selected.tableName} · requested by {selected.requester}
              {selected.total ? ` · total ${selected.total}` : ""}
            </div>
            <dl className="facts">
              {record.fields
                .filter((f) => !f.primary && f.type !== "lifecycle_state" && f.dataType !== "long_text" && f.cell?.d)
                .slice(0, 8)
                .map((f) => (
                  <div key={f.id}>
                    <dt>{f.name}</dt>
                    <dd>{f.cell?.d || "—"}</dd>
                  </div>
                ))}
            </dl>
            <DecisionBox
              run={selected.run}
              task={selected.task}
              canSendBack={selected.canSendBack}
              onDone={(r) => {
                setDone(`“${r.title}”: ${r.process?.approval?.status === "running" ? "your decision is recorded" : r.process?.state.name}`);
                setSelected(null);
                load();
              }}
            />
            <Timeline run={record.process.approval} />
          </div>
        ) : (
          <div className="muted empty">Choose a task to see the request and decide.</div>
        )}
      </div>
    </div>
  );
}

function AllApprovals() {
  const [runs, setRuns] = useState<RunSummary[] | null>(null);
  useEffect(() => {
    api.approvals().then(setRuns).catch(() => setRuns([]));
  }, []);
  if (!runs) return <div className="muted">Loading…</div>;
  return (
    <table className="plain" data-testid="all-approvals">
      <thead>
        <tr><th>#</th><th>Request</th><th>Requested by</th><th>Status</th><th>Waiting for</th><th>Started</th><th>Ended</th></tr>
      </thead>
      <tbody>
        {runs.map((r) => (
          <tr key={r.id}>
            <td>{r.id}</td>
            <td><a href={`#/table/${r.table}`}>{r.title}</a></td>
            <td>{r.requester}</td>
            <td><span className={`run-status run-${r.status.replace(" ", "-")}`}>{r.status}</span></td>
            <td>{r.waitingFor.join(", ")}</td>
            <td>{when(r.startedAt)}</td>
            <td>{when(r.endedAt)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function ApprovalsPage({ onCount }: { onCount: (n: number) => void }) {
  const [tab, setTab] = useState<"tasks" | "all" | "design">("tasks");
  return (
    <div className="page" data-testid="approvals-page">
      <div className="page-head">
        <div>
          <h1>Approvals</h1>
          <p className="muted">What waits for you, every approval you may see, and how the process is designed.</p>
        </div>
      </div>
      <div className="tabs">
        <button className={tab === "tasks" ? "" : "secondary"} onClick={() => setTab("tasks")} data-testid="tab-tasks">My tasks</button>
        <button className={tab === "all" ? "" : "secondary"} onClick={() => setTab("all")} data-testid="tab-all">All approvals</button>
        <button className={tab === "design" ? "" : "secondary"} onClick={() => setTab("design")} data-testid="tab-design">Process design</button>
      </div>
      {tab === "tasks" && <TaskList onCount={onCount} />}
      {tab === "all" && <AllApprovals />}
      {tab === "design" && <ProcessDesign />}
    </div>
  );
}

export function InboxPage() {
  const [notices, setNotices] = useState<Notice[] | null>(null);
  useEffect(() => {
    api.inbox().then(setNotices).catch(() => setNotices([]));
  }, []);
  return (
    <div className="page" data-testid="inbox-page">
      <div className="page-head">
        <div>
          <h1>Inbox</h1>
          <p className="muted">Messages from approvals. Email and chat delivery arrive with mt-notify (wave 5).</p>
        </div>
      </div>
      {!notices ? (
        <div className="muted">Loading…</div>
      ) : notices.length === 0 ? (
        <div className="empty muted">No messages.</div>
      ) : (
        <ul className="inbox" data-testid="inbox">
          {notices.map((n) => (
            <li key={n.id} data-testid="notice">
              <span className="muted small">{when(n.at)}</span>
              <a href={`#/table/${n.table}`}>{n.text}</a>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
