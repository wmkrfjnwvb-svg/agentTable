import { useEffect, useState } from "react";
import { api, Person, setIdentity, TableSummary } from "./api";
import TablePage from "./TablePage";
import { AccessPage, AuditPage, OverviewPage, PlaceholderPage, PLACEHOLDERS, RefDataPage } from "./pages";
import { ApprovalsPage, InboxPage } from "./Process";

function useRoute(): string {
  const [route, setRoute] = useState(() => window.location.hash.slice(1) || "/");
  useEffect(() => {
    const changed = () => setRoute(window.location.hash.slice(1) || "/");
    window.addEventListener("hashchange", changed);
    return () => window.removeEventListener("hashchange", changed);
  }, []);
  return route;
}

const ALICE = "prn_00000000000000000000000001";

function initial(key: string, fallback: string): string {
  try {
    return window.localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

export default function App() {
  const route = useRoute();
  const [people, setPeople] = useState<Person[]>([]);
  const [user, setUser] = useState(() => initial("agenttable.user", ALICE));
  const [onBehalfOf, setOnBehalfOf] = useState(() => initial("agenttable.for", ""));
  const [tables, setTables] = useState<TableSummary[]>([]);
  const [generation, setGeneration] = useState(0);
  const [taskCount, setTaskCount] = useState(0);
  const [noticeCount, setNoticeCount] = useState(0);

  setIdentity(user, onBehalfOf);

  useEffect(() => {
    api.people().then(setPeople).catch(() => setPeople([]));
  }, []);

  useEffect(() => {
    try {
      window.localStorage.setItem("agenttable.user", user);
      window.localStorage.setItem("agenttable.for", onBehalfOf);
    } catch {
      /* private window: nothing to remember */
    }
    api.tables().then(setTables).catch(() => setTables([]));
    setGeneration((g) => g + 1);
  }, [user, onBehalfOf]);

  // Counts in the sidebar, refreshed when the identity or the page changes.
  useEffect(() => {
    api.tasks().then((t) => setTaskCount(t.length)).catch(() => setTaskCount(0));
    api.inbox().then((n) => setNoticeCount(n.length)).catch(() => setNoticeCount(0));
  }, [user, onBehalfOf, route, generation]);

  const me = people.find((p) => p.id === user);
  const isAgent = me?.kind === "agent";
  const humans = people.filter((p) => p.kind === "member" || p.kind === "guest");
  const signIn = people.filter((p) => p.kind !== "group");

  const [section, id] = route.replace(/^\//, "").split("/");
  let page;
  if (section === "table" && id) page = <TablePage key={`${id}-${generation}`} tableId={id} onChanged={() => api.tables().then(setTables)} />;
  else if (section === "refdata") page = <RefDataPage key={generation} />;
  else if (section === "access") page = <AccessPage key={generation} people={people} tables={tables} />;
  else if (section === "audit") page = <AuditPage key={generation} />;
  else if (section === "approvals") page = <ApprovalsPage key={generation} onCount={setTaskCount} />;
  else if (section === "inbox") page = <InboxPage key={generation} />;
  else if (section === "soon" && id) page = <PlaceholderPage which={id} />;
  else page = <OverviewPage key={generation} tables={tables} />;

  const link = (href: string, label: string, extra?: string, testId?: string) => (
    <a href={`#${href}`} className={`nav-item ${route === href ? "active" : ""}`} data-testid={testId}>
      <span>{label}</span>
      {extra && <span className="nav-extra">{extra}</span>}
    </a>
  );

  return (
    <div className="shell">
      <header className="topbar">
        <a href="#/" className="brand">
          <span className="logo" aria-hidden>▦</span> agenttable
          <span className="brand-sub">Purchasing workspace</span>
        </a>
        <div className="identity">
          <label>
            Signed in as
            <select data-testid="user-select" value={user} onChange={(e) => { setUser(e.target.value); if (people.find((p) => p.id === e.target.value)?.kind !== "agent") setOnBehalfOf(""); }}>
              {signIn.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name} — {p.title}
                </option>
              ))}
            </select>
          </label>
          {isAgent && (
            <label>
              acting for
              <select data-testid="behalf-select" value={onBehalfOf} onChange={(e) => setOnBehalfOf(e.target.value)}>
                <option value="">nobody (its own access)</option>
                {humans.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
          )}
          <span className="demo-note" title="Real sign-in (passwords, SSO) arrives with mt-identity in wave 4">demo identities</span>
        </div>
      </header>
      <nav className="sidebar">
        <div className="nav-title">Tables</div>
        {tables.map((t) => (
          <a key={t.id} href={`#/table/${t.id}`} className={`nav-item ${route === `/table/${t.id}` ? "active" : ""} ${t.canRead ? "" : "locked"}`} data-testid={`nav-${t.name}`}>
            <span>{t.canRead ? "▦" : "🔒"} {t.name}</span>
            <span className="nav-extra">{t.canRead ? t.records : ""}</span>
          </a>
        ))}
        <div className="nav-title">Work</div>
        {link("/approvals", "Approvals", taskCount ? `${taskCount}` : undefined, "nav-approvals")}
        {link("/inbox", "Inbox", noticeCount ? `${noticeCount}` : undefined, "nav-inbox")}
        <div className="nav-title">Platform</div>
        {link("/refdata", "Reference data", undefined, "nav-refdata")}
        {link("/access", "Access & explain", undefined, "nav-access")}
        {link("/audit", "Audit log", undefined, "nav-audit")}
        {link("/", "Product status", undefined, "nav-status")}
        <div className="nav-title">Coming next</div>
        {PLACEHOLDERS.map((p) => (
          <a key={p.key} href={`#/soon/${p.key}`} className={`nav-item soon ${route === `/soon/${p.key}` ? "active" : ""}`} data-testid={`nav-soon-${p.key}`}>
            <span>{p.title}</span>
            <span className="badge soon">wave {p.wave}</span>
          </a>
        ))}
      </nav>
      <main className="content">{page}</main>
    </div>
  );
}
