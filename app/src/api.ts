import { invoke } from "@tauri-apps/api/core";

export type SslMode = "disable" | "prefer" | "require" | "verify-ca" | "verify-full";

export type PgTarget = {
  host: string;
  port: number;
  user: string;
  password: string | null;
  database: string;
  ssl: SslMode;
  /** PEM of the CA for the verify modes, when the system doesn't already trust it. */
  ca_cert: string | null;
};

export type Target = ({ engine: "postgres" } & PgTarget) | { engine: "sqlite"; path: string };

/** `agent`: agents may query it read-only through the MCP server. */
export type SavedConnection = { id: string; name: string; target: Target; agent: boolean };

export type RelationKind = "table" | "view" | "materialized_view";

export type RelationRef = { schema: string; name: string };

export type Relation = RelationRef & { kind: RelationKind; estimated_rows: number | null };

export type Column = {
  name: string;
  data_type: string;
  nullable: boolean;
  default: string | null;
  primary_key: boolean;
};

export type Index = { name: string; columns: string[]; unique: boolean; primary: boolean };

export type ForeignKey = {
  name: string | null;
  columns: string[];
  ref_schema: string;
  ref_table: string;
  ref_columns: string[];
};

export type TableInfo = { columns: Column[]; indexes: Index[]; foreign_keys: ForeignKey[] };

export type Sort = { column: string; descending: boolean };

export type FilterOp = "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "contains" | "is_null" | "is_not_null";

export type Filter = { column: string; op: FilterOp; value: string | null };

export type ResultColumn = { name: string; data_type: string };

export type Rows = (string | null)[][];

export type Page = {
  columns: ResultColumn[];
  rows: Rows;
  has_more: boolean;
  elapsed_ms: number;
  sql: string;
};

export type Count = { rows: number | null; exact: boolean };

export type QueryResult = { columns: string[]; rows: Rows; truncated: boolean; elapsed_ms: number };

export type QueryError = { message: string; position: number | null };

export type PlanNode = {
  operation: string;
  target: string | null;
  details: { label: string; value: string }[];
  estimated_rows: number | null;
  actual_rows: number | null;
  loops: number | null;
  total_ms: number | null;
  self_ms: number | null;
  cost: number | null;
  shared_hit: number | null;
  shared_read: number | null;
  children: PlanNode[];
};

export type Finding = { severity: "hot" | "warn"; message: string; path: number[] };

export type Plan = {
  root: PlanNode;
  analyzed: boolean;
  planning_ms: number | null;
  execution_ms: number | null;
  findings: Finding[];
  raw: string;
};

export type Insights = {
  activity: {
    pid: number;
    user: string | null;
    application: string | null;
    state: string | null;
    waiting_on: string | null;
    running_ms: number | null;
    query: string;
    blocked_by: number[];
  }[];
  top_statements: { query: string; calls: number; total_ms: number; mean_ms: number; rows: number }[] | null;
  unused_indexes: { schema: string; table: string; name: string; scans: number; size_bytes: number }[];
  seq_scan_tables: {
    schema: string;
    table: string;
    seq_scans: number;
    seq_rows_read: number;
    index_scans: number | null;
    live_rows: number;
  }[];
};

export type Snippet = { id: string; name: string; sql: string };

export type ColumnValue = { column: string; value: string | null };

/** `old` is what the grid showed; the save only applies if the row still has it. */
export type RowUpdate = { key: ColumnValue[]; changes: { column: string; old: string | null; new: string | null }[] };

export type ExportFormat = "csv" | "tsv" | "json" | "markdown" | "insert";

export type CompletionTable = { schema: string; name: string; columns: string[] };

export type AuditEntry = {
  id: number;
  at_ms: number;
  connection_id: string;
  source: "human" | "agent";
  sql: string;
  elapsed_ms: number;
  rows: number | null;
  error: string | null;
};

export const api = {
  listConnections: () => invoke<SavedConnection[]>("list_connections"),
  hasPassword: (id: string) => invoke<boolean>("has_password", { id }),
  parseUrl: (url: string) => invoke<PgTarget>("parse_url", { url }),
  saveConnection: (connection: SavedConnection, password: string | null) =>
    invoke<SavedConnection>("save_connection", { connection, password }),
  deleteConnection: (id: string) => invoke<void>("delete_connection", { id }),
  testConnection: (connection: SavedConnection, password: string | null) =>
    invoke<string>("test_connection", { connection, password }),
  openSqliteFile: (path: string) => invoke<SavedConnection>("open_sqlite_file", { path }),
  connect: (id: string) => invoke<Relation[]>("connect", { id }),
  disconnect: (id: string) => invoke<void>("disconnect", { id }),
  relations: (id: string) => invoke<Relation[]>("relations", { id }),
  describe: (id: string, relation: RelationRef) => invoke<TableInfo>("describe", { id, relation }),
  count: (id: string, relation: RelationRef, filters: Filter[]) => invoke<Count>("count", { id, relation, filters }),
  page: (
    id: string,
    request: { relation: RelationRef; sort: Sort | null; filters: Filter[]; offset: number; limit: number },
  ) => invoke<Page>("page", { id, request }),
  exportTable: (
    id: string,
    relation: RelationRef,
    sort: Sort | null,
    filters: Filter[],
    format: ExportFormat,
    path: string,
  ) => invoke<number>("export_table", { id, relation, sort, filters, format, path }),
  exportRows: (columns: ResultColumn[], rows: Rows, format: ExportFormat, table: RelationRef | null, path: string) =>
    invoke<void>("export_rows", { columns, rows, format, table, path }),
  copyRows: (columns: ResultColumn[], rows: Rows, format: ExportFormat, table: RelationRef | null) =>
    invoke<string>("copy_rows", { columns, rows, format, table }),
  previewUpdates: (id: string, relation: RelationRef, updates: RowUpdate[]) =>
    invoke<string[]>("preview_updates", { id, relation, updates }),
  applyUpdates: (id: string, relation: RelationRef, updates: RowUpdate[]) =>
    invoke<number>("apply_updates", { id, relation, updates }),
  completionSchema: (id: string) => invoke<CompletionTable[]>("completion_schema", { id }),
  openSession: (connectionId: string) => invoke<string>("open_session", { connectionId }),
  closeSession: (id: string) => invoke<void>("close_session", { id }),
  setWriteMode: (id: string, writable: boolean) => invoke<void>("set_write_mode", { id, writable }),
  cancel: (id: string) => invoke<void>("cancel", { id }),
  runStatement: (id: string, sql: string) => invoke<QueryResult>("run_statement", { id, sql }),
  explain: (id: string, sql: string, analyze: boolean) => invoke<Plan>("explain", { id, sql, analyze }),
  insights: (id: string) => invoke<Insights>("insights", { id }),
  listSnippets: () => invoke<Snippet[]>("list_snippets"),
  saveSnippet: (snippet: Snippet) => invoke<Snippet>("save_snippet", { snippet }),
  deleteSnippet: (id: string) => invoke<void>("delete_snippet", { id }),
  history: (connectionId: string, limit: number) => invoke<AuditEntry[]>("history", { connectionId, limit }),
  agentActivity: (after: number) => invoke<AuditEntry[]>("agent_activity", { after }),
};

export const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

export const plural = (n: number, word: string) => `${n.toLocaleString()} ${n === 1 ? word : `${word}s`}`;

export const sameRelation = (a: RelationRef | null, b: RelationRef | null) =>
  !!a && !!b && a.schema === b.schema && a.name === b.name;

const NUMERIC = /^(smallint|integer|bigint|int|numeric|decimal|real|double|float|money|oid|tinyint|mediumint)/i;
export const isNumeric = (dataType: string) => NUMERIC.test(dataType);

export function compactCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(n < 10_000_000 ? 1 : 0)}M`;
}
