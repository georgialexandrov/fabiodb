import { invoke } from "@tauri-apps/api/core";

export type SslMode = "disable" | "prefer" | "require";

export type PgTarget = {
  host: string;
  port: number;
  user: string;
  password: string | null;
  database: string;
  ssl: SslMode;
};

export type Target = ({ engine: "postgres" } & PgTarget) | { engine: "sqlite"; path: string };

export type SavedConnection = { id: string; name: string; target: Target };

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

export type QueryResult = { columns: string[]; rows: Rows; elapsed_ms: number };

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
  runQuery: (id: string, sql: string) => invoke<QueryResult>("run_query", { id, sql }),
};

export const sameRelation = (a: RelationRef | null, b: RelationRef | null) =>
  !!a && !!b && a.schema === b.schema && a.name === b.name;

const NUMERIC = /^(smallint|integer|bigint|int|numeric|decimal|real|double|float|money|oid|tinyint|mediumint)/i;
export const isNumeric = (dataType: string) => NUMERIC.test(dataType);

export function compactCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(n < 10_000_000 ? 1 : 0)}M`;
}
