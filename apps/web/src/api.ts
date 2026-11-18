// The server API. All business rules (validation, formulas, permissions) live in Rust on the
// server; the web app only shows what the server answers.

export type Value = { type: string; value?: unknown };
export type Cell = { d: string; v?: Value; masked?: boolean; hidden?: boolean; error?: string };

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
  cell?: Cell;
};

export type TableInfo = {
  id: string;
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
};

export type Person = { id: string; name: string; kind: string; title: string; members: string[] };
export type TableSummary = { id: string; name: string; description: string; records: number; canRead: boolean };
export type Option = { value: Value | string; label: string; color?: string; path?: string; depth?: number };

export type ApiError = { status: number; code: string; message: string; details?: unknown };

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
};
