// The server API. All business rules (validation, formulas, permissions) live in Rust on the
// server; the web app only shows what the server answers.

export type Value = { type: string; value?: unknown };
export type Cell = { d: string; v?: Value; items?: string[]; masked?: boolean; hidden?: boolean; error?: string };

export type Column = {
  id: string;
  name: string;
  type: string;
  typeLabel: string;
  group: string;
  dataType: string;
  computed: string | null;
  formula: string | null;
  generated: boolean;
  required: boolean;
  classification: string;
  masked: boolean;
  numeric: boolean;
  primary: boolean;
  linkTarget: string | null;
  fieldGroup: string | null;
  help: string | null;
  maxItems: number | null;
  visibility: "visible" | "masked" | "hidden" | null;
  editable: boolean;
  lockedReason?: string;
  options?: { id: string; label: string; enabled: boolean }[] | null;
  requiredWhen?: { field: string; op: string; text: string } | null;
  requiredWhenText?: string | null;
  cell?: Cell;
};

export type TableInfo = {
  id: string;
  base: string;
  name: string;
  description: string;
  primary: string;
  columns: Column[];
  hidden: { id: string; name: string; reason: string }[];
  canRead: boolean;
  canCreate: boolean;
  canManageSchema: boolean;
};

export type Row = { id: string; title: string; cells: Record<string, Cell>; canEdit: boolean; canDelete: boolean };
export type Group = { key: string; count: number; totals: Record<string, string>; rows: string[] };
export type QueryResult = { table: TableInfo; rows: Row[]; count: number; totals: Record<string, string>; groups: Group[] };

export type Explanation = {
  allowed: boolean;
  chain: {
    grant: string;
    principal: string;
    holder: string;
    viaGroup: boolean;
    capability: string;
    set: string | null;
    on: string;
    scope: string | null;
    reason: string | null;
    grantedBy: string | null;
  }[];
  refusals: { principal: string; grant: string | null; gives: string | null; on: string | null; scope: string | null; reason: string; code: string }[];
  question?: { who: string; for: string | null; capability: string; table: string; record: string | null };
  fields?: string[];
};

export type ApprovalTask = {
  id: number;
  step: string;
  kind: "approve" | "fill_in";
  assignee: string;
  assigneeId: string;
  status: string;
  comment: string | null;
  openedAt: number;
  actedAt: number | null;
  delegatedFrom: string | null;
  mine: boolean;
  canSendBack: boolean;
  fields: { id: string; name: string; dataType: string }[];
  dueAt: number | null;
};

export type ApprovalRun = {
  id: number;
  status: string;
  definition: string | null;
  version: number;
  requester: string;
  startedAt: number;
  endedAt: number | null;
  tasks: ApprovalTask[];
  history: { at: number; actor: string; what: string }[];
};

export type ProcessInfo = {
  state: { key: string; name: string; category: string | null };
  states: { key: string; name: string; category: string }[];
  transitions: { key: string; name: string; to: string; allowed: boolean; reasons: string[]; startsApproval: boolean }[];
  approval: ApprovalRun | null;
  wouldAsk: { steps: { step: string; people: string[] }[]; outcome: string } | null;
};

export type RecordDetail = {
  id: string;
  title: string;
  table: string;
  fields: Column[];
  actions: Record<string, Explanation>;
  createdBy: string;
  createdAt: string;
  modifiedBy: string;
  modifiedAt: string;
  version: number;
  process: ProcessInfo | null;
  canShare: boolean;
};

export type MyTask = {
  run: number;
  task: number;
  kind: "approve" | "fill_in";
  step: string;
  table: string;
  tableName: string | null;
  record: string;
  title: string;
  requester: string;
  total: string | null;
  openedAt: number;
  delegatedFrom: string | null;
  canSendBack: boolean;
  fields: { id: string; name: string; dataType: string }[];
  dueAt: number | null;
};

export type RunSummary = {
  id: number;
  table: string;
  record: string;
  title: string;
  status: string;
  requester: string;
  waitingFor: string[];
  startedAt: number;
  endedAt: number | null;
};

export type Notice = { id: number; at: number; text: string; table: string; record: string };

export type StepNode = {
  kind: "approval" | "branch" | "parallel" | "fill_in" | "notify";
  key: string;
  name: string;
  who?: string[];
  mode?: string;
  canSendBack?: boolean;
  cases?: { name: string; then: StepNode[] }[];
  otherwise?: StepNode[];
  branches?: StepNode[][];
  join?: string;
  fields?: string[];
};

export type ProcessDefinitions = {
  lifecycles: {
    name: string;
    table: string | null;
    initial: string;
    states: { key: string; name: string; category: string; required: string[]; frozen: boolean | null }[];
    transitions: { key: string; name: string; from: string[]; to: string; who: string; approval: string | null }[];
  }[];
  approvals: { key: string; name: string; version: number; steps: StepNode[] }[];
};

export type Act =
  | { act: "approve"; comment?: string }
  | { act: "reject"; comment: string }
  | { act: "return"; comment: string }
  | { act: "submit" }
  | { act: "delegate"; to: string }
  | { act: "add_approver"; who: string }
  | { act: "comment"; text: string };

export type Person = { id: string; name: string; kind: string; title: string; members: string[] };
export type TableSummary = { id: string; base: string; name: string; description: string; records: number; canRead: boolean };
export type BaseSummary = {
  id: string;
  name: string;
  description: string;
  canManageSchema: boolean;
  tables: { id: string; name: string; records: number; canRead: boolean }[];
};
export type Setting = { key: string; type: string; value: string };
export type Template = { key: string; name: string; description: string; installed: boolean };
export type FieldPatch = { name?: string; required?: boolean; help?: string; options?: { id?: string; label: string }[]; requiredWhen?: { field: string; op: string; text: string } | null };
export type Option = { value: Value | string; label: string; color?: string; path?: string; depth?: number };

export type ApiError = { status: number; code: string; message: string; details?: unknown };

export type GrantRow = {
  id: string;
  to: string;
  toId: string;
  what: string;
  on: string;
  scope: string;
  limit: string | null;
  reason: string;
  grantedBy: string;
  active: boolean;
  fields: string | null;
  validFrom: number | null;
  validTo: number | null;
  status: "in_force" | "not_yet" | "ended" | "revoked";
};
export type AccessReview = {
  id: string;
  name: string;
  covers: string;
  state: "open" | "completed" | "cancelled";
  openedBy: string;
  openedAt: number;
  dueAt: number | null;
  overdue: boolean;
  closedBy: string | null;
  done: number;
  total: number;
  items: { grant: string; holder: string; holderId: string; what: GrantRow | null; verdict: "keep" | "revoke" | null; note: string | null; reviewer: string | null }[];
};
export type ShareRow = {
  id: string;
  table: string | null;
  tableId: string | null;
  record: string | null;
  fields: string | null;
  expiresAt: number | null;
  createdBy: string;
  createdAt: number;
  status: "active" | "withdrawn" | "expired" | "not_yet";
  token: string | null;
};
export type SharedView = {
  table: string;
  record: string | null;
  sharedBy: string;
  expiresAt: number | null;
  columns: { id: string; name: string; typeLabel: string; numeric: boolean; primary: boolean }[];
  records: { id: string; title: string; cells: Record<string, Cell> }[];
};

let identity: { user: string; onBehalfOf: string } = { user: "", onBehalfOf: "" };

export function setIdentity(user: string, onBehalfOf: string) {
  identity = { user, onBehalfOf };
}

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {};
  if (identity.user) headers["x-user"] = identity.user;
  if (identity.onBehalfOf) headers["x-on-behalf-of"] = identity.onBehalfOf;
  if (body !== undefined) headers["content-type"] = "application/json";
  const response = await fetch(path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await response.text();
  const data = text ? JSON.parse(text) : null;
  if (!response.ok) {
    const error: ApiError = { status: response.status, code: data?.code ?? "error", message: data?.message ?? response.statusText, details: data?.details };
    throw error;
  }
  return data as T;
}

export type CellInput = { text: string } | { value: Value | null };

export const api = {
  status: () => call<{ product: string; version: string; components: { crate: string; wave: number; state: string; summary: string }[] }>("GET", "/api/status"),
  people: () => call<Person[]>("GET", "/api/people"),
  tables: () => call<TableSummary[]>("GET", "/api/tables"),
  table: (id: string) => call<TableInfo>("GET", `/api/tables/${id}`),
  query: (id: string, spec: unknown) => call<QueryResult>("POST", `/api/tables/${id}/query`, spec),
  record: (table: string, id: string) => call<RecordDetail>("GET", `/api/tables/${table}/records/${id}`),
  create: (table: string, cells: Record<string, CellInput>) => call<RecordDetail>("POST", `/api/tables/${table}/records`, { cells }),
  update: (table: string, id: string, cells: Record<string, CellInput>) => call<RecordDetail>("PATCH", `/api/tables/${table}/records/${id}`, { cells }),
  remove: (table: string, id: string) => call<{ deleted: boolean }>("DELETE", `/api/tables/${table}/records/${id}`),
  options: (table: string, field: string) => call<Option[]>("GET", `/api/tables/${table}/fields/${field}/options`),
  fieldTypes: (q = "", sample = "") =>
    call<{ groups: { key: string; types: { key: string; label: string; group: string; synonyms: string[]; requirements: string[]; unavailable: string | null }[] }[]; search: string[]; suggestion: string | null }>(
      "GET",
      `/api/field-types?q=${encodeURIComponent(q)}&sample=${encodeURIComponent(sample)}`,
    ),
  checkFormula: (table: string, text: string) =>
    call<{ ok: boolean; error?: string; position?: number | null; type?: string; normalized?: string; warnings?: string[]; reads?: string[]; preview?: { record: string; value: string }[] }>(
      "POST",
      `/api/tables/${table}/formula`,
      { text },
    ),
  addField: (table: string, spec: unknown) => call<{ field: string; table: TableInfo }>("POST", `/api/tables/${table}/fields`, spec),
  refsets: () => call<{ id: string; code: string; name: string; description: string | null; tree: boolean; system: boolean; items: number }[]>("GET", "/api/refdata"),
  refset: (id: string, q = "") =>
    call<{ id: string; name: string; description: string | null; tree: boolean; items: { id: string; code: string; name: string; depth: number; path: string; active: boolean; leaf: boolean }[] }>(
      "GET",
      `/api/refdata/${id}?q=${encodeURIComponent(q)}`,
    ),
  capabilities: () => call<{ capabilities: string[]; rules: { name: string; first: string; second: string; kind: string }[] }>("GET", "/api/capabilities"),
  explain: (params: { table: string; record?: string; capability: string; as: string; for?: string }) => {
    const query = new URLSearchParams(Object.entries(params).filter(([, v]) => v) as [string, string][]);
    return call<Explanation>("GET", `/api/explain?${query}`);
  },
  audit: () =>
    call<{ seq: number; hash: string; prev: string; actor: string; initiator: string; action: string; object: string; details: Record<string, unknown> | null; at: string }[]>("GET", "/api/audit"),
  verify: () => call<{ intact: boolean; entries?: number; head?: string; seq?: number; kind?: string; lastGood?: number }>("GET", "/api/audit/verify"),
  tamper: (seq: number) => call<{ intact: boolean; seq?: number; kind?: string; lastGood?: number }>("POST", "/api/audit/tamper-check", { seq }),
  bases: () => call<{ bases: BaseSummary[]; canCreate: boolean }>("GET", "/api/bases"),
  createBase: (name: string, description = "") => call<{ id: string }>("POST", "/api/bases", { name, description }),
  updateBase: (id: string, patch: { name?: string; description?: string }) => call<{ updated: boolean }>("PATCH", `/api/bases/${id}`, patch),
  deleteBase: (id: string) => call<{ deleted: boolean }>("DELETE", `/api/bases/${id}`),
  createTable: (base: string, name: string, primary: string, description = "") => call<TableInfo>("POST", "/api/tables", { base, name, primary, description }),
  updateTable: (id: string, patch: { name?: string; description?: string }) => call<TableInfo>("PATCH", `/api/tables/${id}`, patch),
  deleteTable: (id: string) => call<{ deleted: boolean }>("DELETE", `/api/tables/${id}`),
  updateField: (table: string, field: string, patch: FieldPatch) => call<TableInfo>("PATCH", `/api/tables/${table}/fields/${field}`, patch),
  deleteField: (table: string, field: string) => call<TableInfo>("DELETE", `/api/tables/${table}/fields/${field}`),
  createRefset: (code: string, name: string, tree: boolean) => call<{ id: string }>("POST", "/api/refdata", { code, name, tree }),
  addItem: (set: string, code: string, name: string, parent?: string) => call<{ id: string }>("POST", `/api/refdata/${set}/items`, { code, name, parent: parent || undefined }),
  setItemEnabled: (set: string, item: string, enabled: boolean) => call<{ updated: boolean }>("PATCH", `/api/refdata/${set}/items/${item}`, { enabled }),
  createPerson: (name: string, kind: string, title: string, members: string[] = []) => call<{ id: string }>("POST", "/api/people", { name, kind, title, members }),
  updatePerson: (id: string, patch: { name?: string; title?: string; members?: string[] }) => call<{ updated: boolean }>("PATCH", `/api/people/${id}`, patch),
  settings: () => call<Setting[]>("GET", "/api/settings"),
  setSetting: (key: string, type: string, text: string) => call<{ updated: boolean }>("PUT", `/api/settings/${encodeURIComponent(key)}`, { type, text }),
  processDesign: (table: string) =>
    call<{
      table: { id: string; name: string };
      design: unknown;
      published: { name: string; version: number } | null;
      running: number;
      fields: { id: string; name: string; dataType: string; type: string; generated: boolean; options: { id: string; label: string }[] | null }[];
      canDesign: boolean;
    }>("GET", `/api/tables/${table}/process`),
  publishProcess: (table: string, design: unknown) => call<{ design: unknown }>("PUT", `/api/tables/${table}/process`, design),
  access: () =>
    call<{
      grants: GrantRow[];
      sets: { id: string; name: string; capabilities: string[] }[];
      rules: { name: string; first: string; second: string; kind: string }[];
      reviews: AccessReview[];
      shares: ShareRow[];
      capabilities: string[];
      canManage: boolean;
    }>("GET", "/api/access"),
  giveGrant: (spec: unknown) => call<{ id: string }>("POST", "/api/access/grants", spec),
  revokeGrant: (id: string) => call<{ revoked: boolean }>("DELETE", `/api/access/grants/${id}`),
  openReview: (name: string, on: unknown, dueAt?: number) => call<{ id: string }>("POST", "/api/access/reviews", { name, on, dueAt }),
  decideReview: (review: string, grant: string, verdict: "keep" | "revoke", note?: string) => call<{ decided: boolean }>("POST", `/api/access/reviews/${review}/decisions`, { grant, verdict, note }),
  completeReview: (review: string) => call<{ completed: boolean; revoked: number }>("POST", `/api/access/reviews/${review}/complete`, {}),
  cancelReview: (review: string) => call<{ cancelled: boolean }>("POST", `/api/access/reviews/${review}/cancel`, {}),
  createShare: (table: string, spec: { record?: string; fields?: { except: string[] }; expiresAt?: number }) => call<{ id: string; token: string }>("POST", `/api/tables/${table}/shares`, spec),
  revokeShare: (id: string) => call<{ withdrawn: boolean }>("DELETE", `/api/shares/${id}`),
  openShare: (token: string) => call<SharedView>("GET", `/api/share/${encodeURIComponent(token)}`),
  createSet: (name: string, capabilities: string[]) => call<{ id: string }>("POST", "/api/access/sets", { name, capabilities }),
  addRule: (name: string, first: string, second: string, kind: string) => call<{ id: string }>("POST", "/api/access/rules", { name, first, second, kind }),
  templates: () => call<Template[]>("GET", "/api/templates"),
  installTemplate: (key: string) => call<{ installed: string }>("POST", `/api/templates/${key}/install`),
  transition: (table: string, record: string, key: string) => call<RecordDetail>("POST", `/api/tables/${table}/records/${record}/transitions/${key}`),
  act: (run: number, task: number, act: Act, values?: Record<string, CellInput>) =>
    call<RecordDetail>("POST", `/api/approvals/${run}/tasks/${task}`, values ? { ...act, values } : act),
  tasks: () => call<MyTask[]>("GET", "/api/tasks"),
  approvals: () => call<RunSummary[]>("GET", "/api/approvals"),
  inbox: () => call<Notice[]>("GET", "/api/inbox"),
  process: () => call<ProcessDefinitions>("GET", "/api/process"),
};
